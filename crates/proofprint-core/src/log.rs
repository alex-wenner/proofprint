//! Append-only Merkle log over signed records.
//!
//! The log is the ordering and integrity layer. It is a newline-delimited JSON
//! file — deliberately boring and inspectable, so `Get-Content records.ndjson`
//! is a valid audit tool.
//!
//! ```text
//! <data-dir>/log/records.ndjson     one SignedRecord per line, append-only
//! ```
//!
//! # Tree construction
//!
//! Leaves and internal nodes follow RFC 6962 domain separation (`0x00` prefix
//! for leaves, `0x01` for nodes). The tree is built by pairing adjacent nodes
//! and promoting a trailing odd node, which is equivalent to the RFC's
//! largest-power-of-two split and keeps root computation and proof generation
//! using one code path — so they cannot disagree.
//!
//! # v1 limitations
//!
//! - The whole log is held in memory and the tree is rebuilt on demand. Cost is
//!   O(n) per root/proof call. A production node would maintain an incremental
//!   Merkle mountain range and a SQLite index.
//! - An exclusive OS file lock reserves a data directory for one open log.
//!
//! # Interrupted writes
//!
//! Each append writes its line, newline included, from one buffer and syncs
//! before returning. If the process dies mid-write, the file ends in a line
//! without a newline. On the next open that tail is dropped and the count of
//! discarded bytes is kept in [`Log::recovered_bytes`]. A final line that is
//! complete and verifies but lacks its newline is kept and terminated.
//! Anything else that fails to parse or verify is refused.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::id::RecordId;
use crate::record::SignedRecord;

/// Log file name inside `<data-dir>/log/`.
pub const LOG_FILE: &str = "records.ndjson";

const LEAF_DOMAIN: u8 = 0x00;
const NODE_DOMAIN: u8 = 0x01;

/// RFC 6962 leaf hash over a record's content id.
pub fn leaf_hash(id: &RecordId) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[LEAF_DOMAIN]);
    h.update(id.as_digest());
    *h.finalize().as_bytes()
}

/// RFC 6962 internal node hash.
pub fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[NODE_DOMAIN]);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

/// Build the level above `level`, returning `(next_level, proof_contributions)`.
///
/// `proof_index` is the position of the leaf we are proving; when a pair
/// contains it, the *other* element is the sibling to include. A promoted odd
/// node contributes nothing at its level.
fn build_level(level: &[[u8; 32]], proof_index: Option<usize>) -> (Vec<[u8; 32]>, Vec<[u8; 32]>) {
    let mut next = Vec::with_capacity(level.len().div_ceil(2));
    let mut siblings = Vec::new();
    let mut i = 0;
    while i < level.len() {
        if i + 1 < level.len() {
            if let Some(idx) = proof_index {
                if idx == i || idx == i + 1 {
                    siblings.push(if idx == i { level[i + 1] } else { level[i] });
                }
            }
            next.push(node_hash(&level[i], &level[i + 1]));
            i += 2;
        } else {
            next.push(level[i]);
            i += 1;
        }
    }
    (next, siblings)
}

/// Merkle root of `leaves`, or `None` for an empty tree.
pub fn tree_root(leaves: &[[u8; 32]]) -> Option<[u8; 32]> {
    if leaves.is_empty() {
        return None;
    }
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        level = build_level(&level, None).0;
    }
    Some(level[0])
}

/// Build the inclusion proof path for `index` in a tree over `leaves`.
pub fn build_inclusion_path(leaves: &[[u8; 32]], index: usize) -> Result<Vec<[u8; 32]>> {
    if index >= leaves.len() {
        return Err(Error::BadInclusionProof);
    }
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    let mut idx = index;
    let mut path = Vec::new();
    while level.len() > 1 {
        let (next, siblings) = build_level(&level, Some(idx));
        path.extend(siblings);
        level = next;
        idx /= 2;
    }
    Ok(path)
}

/// Check an inclusion proof against a root.
///
/// Walk each tree level, consuming a sibling only when one exists. A trailing
/// unpaired node is promoted without consuming a proof element.
pub fn verify_inclusion(
    leaf_index: u64,
    tree_size: u64,
    leaf: &[u8; 32],
    path: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    if tree_size == 0 || leaf_index >= tree_size {
        return false;
    }
    let mut hash = *leaf;
    let mut idx = leaf_index;
    let mut width = tree_size;
    let mut siblings = path.iter();
    while width > 1 {
        if idx % 2 == 1 {
            let Some(sibling) = siblings.next() else {
                return false;
            };
            hash = node_hash(sibling, &hash);
        } else if idx + 1 < width {
            let Some(sibling) = siblings.next() else {
                return false;
            };
            hash = node_hash(&hash, sibling);
        }
        idx /= 2;
        width = width.div_ceil(2);
    }
    siblings.next().is_none() && hash == *root
}

/// Everything needed to independently check that a record is in a log state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    /// Zero-based position of the record.
    pub leaf_index: u64,
    /// Number of records when the proof was produced.
    pub tree_size: u64,
    /// Hash of the leaf being proved.
    pub leaf_hash: [u8; 32],
    /// Sibling hashes from leaf to root.
    pub path: Vec<[u8; 32]>,
    /// Root of the tree at `tree_size`.
    pub root: [u8; 32],
}

impl InclusionProof {
    /// Re-derive the root from the proof and confirm it matches.
    pub fn verify(&self) -> Result<()> {
        if verify_inclusion(
            self.leaf_index,
            self.tree_size,
            &self.leaf_hash,
            &self.path,
            &self.root,
        ) {
            Ok(())
        } else {
            Err(Error::BadInclusionProof)
        }
    }
}

/// Controls applied when appending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppendOptions {
    /// Reject a record whose declared parents are not already in the log.
    ///
    /// Enforces local parent availability. Turn off when importing history whose
    /// ancestors live elsewhere.
    pub require_parents_present: bool,
}

impl Default for AppendOptions {
    fn default() -> Self {
        Self {
            require_parents_present: true,
        }
    }
}

/// An in-memory view of the append-only log, backed by `records.ndjson`.
#[derive(Debug)]
pub struct Log {
    _lock: File,
    path: PathBuf,
    entries: Vec<SignedRecord>,
    ids: Vec<RecordId>,
    positions: HashMap<RecordId, usize>,
    leaves: Vec<[u8; 32]>,
    recovered_bytes: u64,
}

impl Log {
    /// Open or create the log under `<data-dir>/log/`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = data_dir.as_ref().join("log");
        fs::create_dir_all(&dir)?;
        let lock_path = dir.join("writer.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                Error::LedgerInUse(lock_path)
            } else {
                Error::Io(error)
            }
        })?;
        let path = dir.join(LOG_FILE);
        let mut log = Self {
            _lock: lock,
            path,
            entries: Vec::new(),
            ids: Vec::new(),
            positions: HashMap::new(),
            leaves: Vec::new(),
            recovered_bytes: 0,
        };
        log.load()?;
        Ok(log)
    }

    /// Path to the NDJSON file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bytes of an interrupted final write discarded when this log was opened.
    pub fn recovered_bytes(&self) -> u64 {
        self.recovered_bytes
    }

    fn load(&mut self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut reader = BufReader::new(File::open(&self.path)?);
        let mut line = Vec::new();
        let mut offset = 0u64;
        let mut number = 0usize;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                return Ok(());
            }
            number += 1;
            if line.last() != Some(&b'\n') {
                return self.load_tail(&line, offset);
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                let signed = parse_line(&line)
                    .map_err(|e| Error::MalformedRecord(format!("{LOG_FILE}:{number}: {e}")))?;
                // Re-verify on load: a record that no longer verifies means the
                // log has been tampered with, and we must not silently serve it.
                let id = signed.verify()?;
                self.push_entry(id, signed);
            }
            offset += read as u64;
        }
    }

    /// Handle a final line with no newline: keep it if complete, drop it if torn.
    fn load_tail(&mut self, line: &[u8], offset: u64) -> Result<()> {
        match parse_line(line) {
            Ok(signed) => {
                let id = signed.verify()?;
                self.push_entry(id, signed);
                let mut file = OpenOptions::new().append(true).open(&self.path)?;
                file.write_all(b"\n")?;
                file.sync_all()?;
            }
            Err(_) => {
                let file = OpenOptions::new().write(true).open(&self.path)?;
                file.set_len(offset)?;
                file.sync_all()?;
                self.recovered_bytes = line.len() as u64;
            }
        }
        Ok(())
    }

    fn push_entry(&mut self, id: RecordId, signed: SignedRecord) {
        let pos = self.entries.len();
        self.entries.push(signed);
        self.ids.push(id);
        self.leaves.push(leaf_hash(&id));
        self.positions.insert(id, pos);
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the log holds no records.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Verify and append a record. Returns its zero-based position.
    ///
    /// Appending an already-present id is a no-op that returns its position, so
    /// publish is idempotent.
    pub fn append_with(&mut self, signed: SignedRecord, opts: AppendOptions) -> Result<usize> {
        let id = signed.verify()?;

        if let Some(pos) = self.positions.get(&id) {
            return Ok(*pos);
        }

        if opts.require_parents_present {
            for parent in &signed.record.parents {
                if !self.positions.contains_key(parent) {
                    return Err(Error::ParentNotFound(parent.to_string()));
                }
            }
        }

        let mut line = serde_json::to_vec(&signed)?;
        line.push(b'\n');
        {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            file.write_all(&line)?;
            file.sync_all()?;
        }

        self.push_entry(id, signed);
        Ok(self.entries.len() - 1)
    }

    /// Verify and append with default options.
    pub fn append(&mut self, signed: SignedRecord) -> Result<usize> {
        self.append_with(signed, AppendOptions::default())
    }

    /// Current Merkle root, or `None` if the log is empty.
    pub fn root(&self) -> Option<[u8; 32]> {
        tree_root(&self.leaves)
    }

    /// Current root as hex.
    pub fn root_hex(&self) -> Option<String> {
        self.root().map(hex::encode)
    }

    /// Look up a record by id.
    pub fn get(&self, id: &RecordId) -> Option<&SignedRecord> {
        self.positions.get(id).map(|p| &self.entries[*p])
    }

    /// Zero-based position of a record.
    pub fn position_of(&self, id: &RecordId) -> Option<usize> {
        self.positions.get(id).copied()
    }

    /// All records in log order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, RecordId, &SignedRecord)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, r)| (i, self.ids[i], r))
    }

    /// Records of a given `kind`, in log order.
    pub fn by_kind(&self, kind: &str) -> Vec<(usize, RecordId, &SignedRecord)> {
        self.iter()
            .filter(|(_, _, r)| r.record.kind == kind)
            .collect()
    }

    /// Produce an inclusion proof for `id`.
    pub fn inclusion_proof(&self, id: &RecordId) -> Result<InclusionProof> {
        let pos = self
            .positions
            .get(id)
            .copied()
            .ok_or_else(|| Error::RecordNotFound(id.to_string()))?;
        let root = self.root().ok_or(Error::BadInclusionProof)?;
        Ok(InclusionProof {
            leaf_index: pos as u64,
            tree_size: self.entries.len() as u64,
            leaf_hash: self.leaves[pos],
            path: build_inclusion_path(&self.leaves, pos)?,
            root,
        })
    }

    /// Direct children — records that name `id` as a parent.
    pub fn children_of(&self, id: &RecordId) -> Vec<RecordId> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, r)| r.record.parents.contains(id))
            .map(|(i, _)| self.ids[i])
            .collect()
    }

    /// Transitive ancestors in traversal order, deduplicated.
    pub fn ancestors_of(&self, id: &RecordId) -> Vec<RecordId> {
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        let mut stack = vec![*id];
        while let Some(cur) = stack.pop() {
            let Some(rec) = self.get(&cur) else { continue };
            for p in &rec.record.parents {
                if seen.insert(*p, ()).is_none() {
                    out.push(*p);
                    stack.push(*p);
                }
            }
        }
        out
    }

    /// Transitive descendants.
    pub fn descendants_of(&self, id: &RecordId) -> Vec<RecordId> {
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        let mut stack = vec![*id];
        while let Some(cur) = stack.pop() {
            for c in self.children_of(&cur) {
                if seen.insert(c, ()).is_none() {
                    out.push(c);
                    stack.push(c);
                }
            }
        }
        out
    }
}

fn parse_line(line: &[u8]) -> std::result::Result<SignedRecord, String> {
    let text = std::str::from_utf8(line).map_err(|e| e.to_string())?;
    serde_json::from_str(text.trim()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyPair;
    use crate::record::Record;
    use serde_json::json;

    fn leaf(b: &[u8]) -> [u8; 32] {
        leaf_hash(&RecordId::from_bytes(b))
    }

    #[test]
    fn empty_tree_has_no_root() {
        assert_eq!(tree_root(&[]), None);
    }

    #[test]
    fn only_one_log_can_own_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = Log::open(dir.path()).unwrap();
        assert!(matches!(Log::open(dir.path()), Err(Error::LedgerInUse(_))));
        drop(first);
        assert!(Log::open(dir.path()).is_ok());
    }

    #[test]
    fn single_leaf_root_is_leaf_hash() {
        let l = leaf(b"a");
        assert_eq!(tree_root(&[l]), Some(l));
    }

    #[test]
    fn two_leaf_root() {
        let (a, b) = (leaf(b"a"), leaf(b"b"));
        assert_eq!(tree_root(&[a, b]), Some(node_hash(&a, &b)));
    }

    #[test]
    fn proofs_verify_for_every_index_and_size() {
        for n in 1..=40usize {
            let leaves: Vec<[u8; 32]> = (0..n).map(|i| leaf(&i.to_le_bytes())).collect();
            let root = tree_root(&leaves).unwrap();
            for i in 0..n {
                let path = build_inclusion_path(&leaves, i).unwrap();
                assert!(
                    verify_inclusion(i as u64, n as u64, &leaves[i], &path, &root),
                    "failed at n={n} i={i}"
                );
            }
        }
    }

    #[test]
    fn proof_rejects_wrong_leaf() {
        let leaves: Vec<[u8; 32]> = (0u64..8).map(|i| leaf(&i.to_le_bytes())).collect();
        let root = tree_root(&leaves).unwrap();
        let path = build_inclusion_path(&leaves, 3).unwrap();
        assert!(!verify_inclusion(3, 8, &leaf(b"other"), &path, &root));
    }

    #[test]
    fn proof_rejects_wrong_index_or_root() {
        let leaves: Vec<[u8; 32]> = (0u64..8).map(|i| leaf(&i.to_le_bytes())).collect();
        let root = tree_root(&leaves).unwrap();
        let path = build_inclusion_path(&leaves, 3).unwrap();
        assert!(!verify_inclusion(4, 8, &leaves[3], &path, &root));
        assert!(!verify_inclusion(
            3,
            8,
            &leaves[3],
            &path,
            &leaf(b"fake-root")
        ));
        assert!(!verify_inclusion(8, 8, &leaves[3], &path, &root));
    }

    #[test]
    fn proof_rejects_truncation_and_extension() {
        let leaves: Vec<[u8; 32]> = (0u64..16).map(|i| leaf(&i.to_le_bytes())).collect();
        let root = tree_root(&leaves).unwrap();
        let path = build_inclusion_path(&leaves, 9).unwrap();
        assert!(!verify_inclusion(
            9,
            16,
            &leaves[9],
            &path[..path.len() - 1],
            &root
        ));
        let mut extended = path.clone();
        extended.push(leaf(b"extra"));
        assert!(!verify_inclusion(9, 16, &leaves[9], &extended, &root));
    }

    #[test]
    fn out_of_range_path_is_rejected() {
        let leaves = vec![leaf(b"a")];
        assert!(matches!(
            build_inclusion_path(&leaves, 1),
            Err(Error::BadInclusionProof)
        ));
    }

    // ---- Log ----

    fn open_log() -> (tempfile::TempDir, Log, KeyPair) {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::open(dir.path()).unwrap();
        (dir, log, KeyPair::generate())
    }

    fn rec(kp: &KeyPair, step: u64, parents: Vec<RecordId>) -> SignedRecord {
        Record::new("ml.training-step/v1", json!({"step": step}), kp.signer())
            .with_parents(parents)
            .sign(kp)
            .unwrap()
    }

    #[test]
    fn append_then_lookup() {
        let (_d, mut log, kp) = open_log();
        let r = rec(&kp, 1, vec![]);
        let pos = log.append(r.clone()).unwrap();
        assert_eq!(pos, 0);
        assert_eq!(log.len(), 1);
        let id = r.id().unwrap();
        assert_eq!(log.get(&id).unwrap(), &r);
        assert_eq!(log.root_hex().unwrap().len(), 64);
    }

    #[test]
    fn append_is_idempotent() {
        let (_d, mut log, kp) = open_log();
        let r = rec(&kp, 1, vec![]);
        let a = log.append(r.clone()).unwrap();
        let b = log.append(r).unwrap();
        assert_eq!(a, b);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn append_rejects_unsigned_or_tampered_records() {
        let (_d, mut log, kp) = open_log();
        let mut r = rec(&kp, 1, vec![]);
        r.record.payload = json!({"step": 999});
        assert!(matches!(log.append(r), Err(Error::BadSignature)));
    }

    #[test]
    fn missing_parent_is_rejected_unless_allowed() {
        let (_d, mut log, kp) = open_log();
        let orphan = rec(&kp, 1, vec![RecordId::from_bytes(b"ghost")]);
        assert!(matches!(
            log.append(orphan.clone()),
            Err(Error::ParentNotFound(_))
        ));
        let opts = AppendOptions {
            require_parents_present: false,
        };
        assert!(log.append_with(orphan, opts).is_ok());
    }

    #[test]
    fn log_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        let id;
        {
            let mut log = Log::open(dir.path()).unwrap();
            let r = rec(&kp, 1, vec![]);
            id = r.id().unwrap();
            log.append(r).unwrap();
            log.append(rec(&kp, 2, vec![id])).unwrap();
        }
        let reopened = Log::open(dir.path()).unwrap();
        assert_eq!(reopened.len(), 2);
        assert!(reopened.get(&id).is_some());
    }

    #[test]
    fn reopen_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        {
            let mut log = Log::open(dir.path()).unwrap();
            log.append(rec(&kp, 1, vec![])).unwrap();
        }
        let path = dir.path().join("log").join(LOG_FILE);
        let content = fs::read_to_string(&path).unwrap();
        let tampered = content.replace("\"step\":1", "\"step\":42");
        assert_ne!(content, tampered, "fixture did not contain step:1");
        fs::write(&path, tampered).unwrap();
        assert!(Log::open(dir.path()).is_err());
    }

    #[test]
    fn torn_final_line_is_dropped_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        {
            let mut log = Log::open(dir.path()).unwrap();
            log.append(rec(&kp, 1, vec![])).unwrap();
        }
        let path = dir.path().join("log").join(LOG_FILE);
        let intact = fs::read(&path).unwrap();
        let torn = br#"{"record":{"v":1,"kind":"ml.tra"#;
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(torn).unwrap();
        drop(file);

        let mut log = Log::open(dir.path()).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log.recovered_bytes(), torn.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), intact);

        log.append(rec(&kp, 2, vec![])).unwrap();
        drop(log);
        assert_eq!(Log::open(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn complete_final_line_without_newline_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        let first = rec(&kp, 1, vec![]);
        let dir_log = dir.path().join("log");
        fs::create_dir_all(&dir_log).unwrap();
        let path = dir_log.join(LOG_FILE);
        fs::write(&path, serde_json::to_string(&first).unwrap()).unwrap();

        let mut log = Log::open(dir.path()).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log.recovered_bytes(), 0);
        assert!(fs::read(&path).unwrap().ends_with(b"\n"));

        log.append(rec(&kp, 2, vec![])).unwrap();
        drop(log);
        assert_eq!(Log::open(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn torn_line_that_fails_verification_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        let mut forged = rec(&kp, 1, vec![]);
        forged.record.payload = json!({"step": 2});
        let dir_log = dir.path().join("log");
        fs::create_dir_all(&dir_log).unwrap();
        let path = dir_log.join(LOG_FILE);
        fs::write(&path, serde_json::to_string(&forged).unwrap()).unwrap();
        assert!(matches!(Log::open(dir.path()), Err(Error::BadSignature)));
    }

    #[test]
    fn inclusion_proof_round_trips_through_log() {
        let (_d, mut log, kp) = open_log();
        let mut prev = None;
        let mut target = None;
        for step in 1..=12u64 {
            let r = rec(&kp, step, prev.into_iter().collect());
            let id = r.id().unwrap();
            log.append(r).unwrap();
            if step == 7 {
                target = Some(id);
            }
            prev = Some(id);
        }
        let proof = log.inclusion_proof(&target.unwrap()).unwrap();
        assert_eq!(proof.tree_size, 12);
        assert!(proof.verify().is_ok());
    }

    #[test]
    fn inclusion_proof_rejects_unknown_record() {
        let (_d, log, _kp) = open_log();
        assert!(matches!(
            log.inclusion_proof(&RecordId::from_bytes(b"nope")),
            Err(Error::RecordNotFound(_))
        ));
    }

    #[test]
    fn lineage_traversal() {
        let (_d, mut log, kp) = open_log();
        let root = rec(&kp, 1, vec![]);
        let root_id = root.id().unwrap();
        log.append(root).unwrap();

        let mid = rec(&kp, 2, vec![root_id]);
        let mid_id = mid.id().unwrap();
        log.append(mid).unwrap();

        let leaf = rec(&kp, 3, vec![mid_id]);
        let leaf_id = leaf.id().unwrap();
        log.append(leaf).unwrap();

        assert_eq!(log.children_of(&root_id), vec![mid_id]);
        assert_eq!(log.descendants_of(&root_id), vec![mid_id, leaf_id]);
        let anc = log.ancestors_of(&leaf_id);
        assert!(anc.contains(&root_id) && anc.contains(&mid_id));
        assert_eq!(anc.len(), 2);
    }

    #[test]
    fn by_kind_filters() {
        let (_d, mut log, kp) = open_log();
        log.append(rec(&kp, 1, vec![])).unwrap();
        let other = Record::new("software.release/v1", json!({"tag": "v0.1"}), kp.signer())
            .sign(&kp)
            .unwrap();
        log.append(other).unwrap();
        assert_eq!(log.by_kind("ml.training-step/v1").len(), 1);
        assert_eq!(log.by_kind("software.release/v1").len(), 1);
        assert_eq!(log.by_kind("nope/v1").len(), 0);
    }

    #[test]
    fn diamond_lineage_dedupes_ancestors() {
        let (_d, mut log, kp) = open_log();
        let base = rec(&kp, 0, vec![]);
        let base_id = base.id().unwrap();
        log.append(base).unwrap();

        let l = rec(&kp, 1, vec![base_id]);
        let l_id = l.id().unwrap();
        log.append(l).unwrap();
        let r = rec(&kp, 2, vec![base_id]);
        let r_id = r.id().unwrap();
        log.append(r).unwrap();

        let merge = rec(&kp, 3, vec![l_id, r_id]);
        let merge_id = merge.id().unwrap();
        log.append(merge).unwrap();

        let anc = log.ancestors_of(&merge_id);
        assert_eq!(anc.len(), 3, "{anc:?}");
        assert!(anc.contains(&base_id));
    }
}
