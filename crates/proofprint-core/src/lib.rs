//! # ProofPrint
//!
//! A **general-purpose audit ledger**: signed records, content-addressed
//! artifacts, and an append-only Merkle log that makes the resulting history
//! independently auditable.
//!
//! The motivating problem is that claims about how a model was trained — which
//! data, which recipe, which starting state — are currently unverifiable
//! assertions. ProofPrint does not make those assertions *true*. It makes them
//! **committed, attributable, and ordered**. Independent observers can retain
//! those commitments and compare them with subsequent records.
//!
//! ## What this crate guarantees
//!
//! - **Integrity**: a record's id is the hash of its canonical encoding, so any
//!   change produces a different id.
//! - **Attribution**: every record carries an Ed25519 signature over exactly
//!   those bytes.
//! - **Ordering**: records live in an append-only log with an RFC 6962 Merkle
//!   root and inclusion proofs.
//! - **Lineage**: records reference parents by id, forming a DAG.
//! - **Content addressing**: large artifacts are stored by hash and referenced,
//!   never embedded.
//!
//! ## What it does not guarantee
//!
//! A valid signature proves authorship, not truth. "Company A says it trained on
//! dataset D" and "Company A trained on dataset D" are different statements.
//! Closing that gap needs domain auditors — see [`schema::Verification`] — and
//! ultimately depends on the cost of being caught lying in a permanent public
//! record.
//!
//! ## Shape
//!
//! ```text
//! Ledger
//! ├── BlobStore   content-addressed bytes
//! └── Log         append-only signed records + Merkle tree
//! ```
//!
//! ## Example
//!
//! ```
//! use proofprint_core::{Ledger, Record};
//! use proofprint_core::key::KeyPair;
//! use proofprint_core::schema::{Schema, TrainingStep};
//! use serde_json::json;
//!
//! # fn main() -> proofprint_core::Result<()> {
//! let dir = tempfile::tempdir().unwrap();
//! let mut ledger = Ledger::open(dir.path())?;
//! let keys = KeyPair::generate();
//!
//! // A record built from a typed schema.
//! let step = TrainingStep {
//!     step: 1,
//!     samples_seen: Some(2048),
//!     loss: Some(8.31),
//!     metrics: json!({"grad_norm": 2.1}),
//!     checkpoint_blob: None,
//! };
//! let run = step.to_record(keys.signer())?.sign(&keys)?;
//! let run_id = ledger.append(run)?;
//!
//! // A record built ad hoc — the ledger does not care what the payload means.
//! let note = Record::new("example.note/v1", json!({"text": "hello"}), keys.signer());
//! let note_id = ledger.append(note.sign(&keys)?)?;
//!
//! // Both are now permanently ordered and individually provable.
//! let proof = ledger.log.inclusion_proof(&note_id)?;
//! proof.verify()?;
//! assert_eq!(ledger.log.len(), 2);
//! assert_eq!(run_id.to_string().len(), 69);
//! # Ok(())
//! # }
//! ```

pub mod canonical;
pub mod continuity;
pub mod error;
pub mod id;
pub mod key;
pub mod log;
pub mod record;
pub mod schema;
pub mod store;
pub mod weights;

pub use error::{Error, Result};
pub use id::{BlobId, RecordId};
pub use key::{KeyPair, PublicKey};
pub use log::{InclusionProof, Log};
pub use record::{BlobRef, Record, SignedRecord};
pub use store::BlobStore;

use std::path::{Path, PathBuf};

/// A data directory containing a [`BlobStore`] and a [`Log`].
///
/// This is the unit most tools operate on. It is not a network — it is one
/// node's local view. Replication and consensus are separate layers.
#[derive(Debug)]
pub struct Ledger {
    /// Root of this node's data directory.
    pub data_dir: PathBuf,
    /// Content-addressed artifact storage.
    pub blobs: BlobStore,
    /// Append-only record log.
    pub log: Log,
}

impl Ledger {
    /// Open or create a ledger rooted at `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;
        Ok(Self {
            blobs: BlobStore::open(&data_dir)?,
            log: Log::open(&data_dir)?,
            data_dir,
        })
    }

    /// Verify and append an already-signed record. Returns its id.
    pub fn append(&mut self, signed: SignedRecord) -> Result<RecordId> {
        let id = signed.id()?;
        self.log.append(signed)?;
        Ok(id)
    }

    /// Verify and append with explicit options. Returns its id.
    pub fn append_with(
        &mut self,
        signed: SignedRecord,
        opts: log::AppendOptions,
    ) -> Result<RecordId> {
        let id = signed.id()?;
        self.log.append_with(signed, opts)?;
        Ok(id)
    }

    /// Sign and append a record. Returns its id.
    pub fn publish(&mut self, record: Record, keys: &KeyPair) -> Result<RecordId> {
        self.append(record.sign(keys)?)
    }

    /// Stream a file into blob storage as a named attachment.
    pub fn attach_file(
        &self,
        name: impl Into<String>,
        path: impl AsRef<Path>,
        media: Option<String>,
    ) -> Result<BlobRef> {
        self.blobs.put_file(name, path, media)
    }

    /// Store bytes as a named attachment.
    pub fn attach_bytes(
        &self,
        name: impl Into<String>,
        bytes: &[u8],
        media: Option<String>,
    ) -> Result<BlobRef> {
        self.blobs.put_named(name, bytes, media)
    }

    /// Which of a record's declared blobs are present locally.
    pub fn blob_availability(&self, record: &Record) -> Vec<(String, bool)> {
        record
            .blobs
            .iter()
            .map(|b| (b.name.clone(), self.blobs.contains(&b.blob)))
            .collect()
    }

    /// Re-hash a stored blob and confirm it matches its declared id.
    ///
    /// Catches storage corruption and substitution. Returns the verified size.
    pub fn verify_blob(&self, blob: &BlobRef) -> Result<u64> {
        let size = self.blobs.verify(&blob.blob)?;
        if size != blob.size {
            return Err(Error::MalformedRecord(format!(
                "blob {} declares {} bytes but {} are stored",
                blob.name, blob.size, size
            )));
        }
        Ok(size)
    }

    /// Verify every blob a record references that is present locally.
    ///
    /// Missing blobs are reported in the returned list rather than treated as
    /// failures, since a node is not required to mirror the whole archive.
    pub fn verify_record_blobs(&self, record: &Record) -> Result<BlobAudit> {
        let mut audit = BlobAudit::default();
        for b in &record.blobs {
            if !self.blobs.contains(&b.blob) {
                audit.missing.push(b.name.clone());
                continue;
            }
            match self.verify_blob(b) {
                Ok(size) => {
                    audit.verified_bytes += size;
                    audit.verified.push(b.name.clone());
                }
                Err(e) => audit.corrupt.push((b.name.clone(), e.to_string())),
            }
        }
        Ok(audit)
    }
}

/// Outcome of checking a record's attachments against blob storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlobAudit {
    /// Names whose bytes were present and hashed correctly.
    pub verified: Vec<String>,
    /// Names whose bytes are not stored locally.
    pub missing: Vec<String>,
    /// Names whose stored bytes did not match the declared id or size.
    pub corrupt: Vec<(String, String)>,
    /// Total bytes verified.
    pub verified_bytes: u64,
}

impl BlobAudit {
    /// True when nothing was corrupt.
    pub fn is_consistent(&self) -> bool {
        self.corrupt.is_empty()
    }

    /// True when every referenced blob was present and consistent.
    pub fn is_complete(&self) -> bool {
        self.corrupt.is_empty() && self.missing.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use record::utc;
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, Ledger, KeyPair) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path()).unwrap();
        (dir, ledger, KeyPair::generate())
    }

    #[test]
    fn open_creates_layout() {
        let dir = tempfile::tempdir().unwrap();
        let l = Ledger::open(dir.path()).unwrap();
        assert!(dir.path().join("blobs").is_dir());
        assert!(dir.path().join("log").is_dir());
        assert_eq!(l.data_dir, dir.path());
    }

    #[test]
    fn publish_and_retrieve() {
        let (_d, mut l, kp) = setup();
        let rec = Record::new("example.note/v1", json!({"n": 1}), kp.signer()).with_created(utc(0));
        let id = l.publish(rec, &kp).unwrap();
        assert!(l.log.get(&id).is_some());
        assert_eq!(l.log.len(), 1);
    }

    #[test]
    fn lineage_is_queryable() {
        let (_d, mut l, kp) = setup();
        let a = l
            .publish(
                Record::new("example.note/v1", json!({"n": 1}), kp.signer()).with_created(utc(1)),
                &kp,
            )
            .unwrap();
        let b = l
            .publish(
                Record::new("example.note/v1", json!({"n": 2}), kp.signer())
                    .with_created(utc(2))
                    .with_parent(a),
                &kp,
            )
            .unwrap();
        assert_eq!(l.log.children_of(&a), vec![b]);
        assert_eq!(l.log.ancestors_of(&b), vec![a]);
    }

    #[test]
    fn attach_and_verify_bytes() {
        let (_d, mut l, kp) = setup();
        let b = l.attach_bytes("weights", b"fake-weights", None).unwrap();
        let rec = Record::new("ml.model/v1", json!({"name": "m"}), kp.signer())
            .with_created(utc(0))
            .with_blob(b);
        let id = l.publish(rec, &kp).unwrap();

        let stored = l.log.get(&id).unwrap();
        let audit = l.verify_record_blobs(&stored.record).unwrap();
        assert!(audit.is_complete(), "{audit:?}");
        assert_eq!(audit.verified, vec!["weights"]);
        assert_eq!(audit.verified_bytes, 12);
    }

    #[test]
    fn verify_blob_detects_substitution() {
        let (_d, l, _kp) = setup();
        let b = l.attach_bytes("data", b"original", None).unwrap();
        assert_eq!(l.verify_blob(&b).unwrap(), 8);

        let path = l.blobs.path_for(&b.blob);
        std::fs::write(&path, b"tampered!!").unwrap();
        assert!(l.verify_blob(&b).is_err());
    }

    #[test]
    fn verify_blob_detects_size_mismatch() {
        let (_d, l, _kp) = setup();
        let mut b = l.attach_bytes("data", b"12345", None).unwrap();
        b.size = 999;
        assert!(l.verify_blob(&b).is_err());
    }

    #[test]
    fn missing_blobs_are_reported_not_fatal() {
        let (_d, l, kp) = setup();
        let phantom = BlobRef {
            name: "elsewhere".into(),
            blob: BlobId::from_bytes(b"not-stored-here"),
            size: 10,
            media: None,
        };
        let rec = Record::new("ml.model/v1", json!({}), kp.signer())
            .with_created(utc(0))
            .with_blob(phantom);
        let audit = l.verify_record_blobs(&rec).unwrap();
        assert_eq!(audit.missing, vec!["elsewhere"]);
        assert!(audit.is_consistent());
        assert!(!audit.is_complete());
    }

    #[test]
    fn blob_availability_flags_presence() {
        let (_d, l, _kp) = setup();
        let present = l.attach_bytes("a", b"aaa", None).unwrap();
        let absent = BlobRef {
            name: "b".into(),
            blob: BlobId::from_bytes(b"bbb"),
            size: 3,
            media: None,
        };
        let rec = Record {
            v: record::PROTOCOL_VERSION,
            kind: "x.y/v1".into(),
            parents: vec![],
            payload: json!({}),
            blobs: vec![present, absent],
            created: utc(0),
            signer: "ed25519:00".into(),
        };
        let avail = l.blob_availability(&rec);
        assert_eq!(
            avail,
            vec![("a".to_string(), true), ("b".to_string(), false)]
        );
    }

    #[test]
    fn ledger_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let kp = KeyPair::generate();
        let id = {
            let mut l = Ledger::open(dir.path()).unwrap();
            l.attach_bytes("weights", b"w", None).unwrap();
            l.publish(
                Record::new("example.note/v1", json!({}), kp.signer()).with_created(utc(0)),
                &kp,
            )
            .unwrap()
        };
        let l = Ledger::open(dir.path()).unwrap();
        assert_eq!(l.log.len(), 1);
        assert!(l.log.get(&id).is_some());
        assert_eq!(l.blobs.count().unwrap(), 1);
    }
}
