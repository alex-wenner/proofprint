//! One ledger behind a lock, an optional signing key, and typed answers.
//!
//! Every method here blocks briefly. Handlers call them through
//! [`Node::run`], which moves the work off the async executor.

use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, SecondsFormat, Utc};
use proofprint_core::{BlobRef, BlobStore, KeyPair, Ledger, Record, RecordId, SignedRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::NodeError;

type Result<T> = std::result::Result<T, NodeError>;

const DEFAULT_PAGE: usize = 200;
const MAX_PAGE: usize = 1000;

/// Shared state of a running node.
pub struct Node {
    ledger: Mutex<Ledger>,
    keys: Option<KeyPair>,
}

impl Node {
    /// Wrap an open ledger. Without `keys`, drafts are refused and only
    /// already-signed records are accepted.
    pub fn new(ledger: Ledger, keys: Option<KeyPair>) -> Self {
        Self {
            ledger: Mutex::new(ledger),
            keys,
        }
    }

    /// Run a blocking operation on a worker thread.
    pub async fn run<T, F>(self: Arc<Self>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Node) -> Result<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(move || operation(&self))
            .await
            .map_err(NodeError::internal)?
    }

    /// Public identifier of the signing key, if one was loaded.
    pub fn signer(&self) -> Option<String> {
        self.keys.as_ref().map(KeyPair::signer)
    }

    /// A handle to artifact storage that streams without holding the ledger lock.
    pub fn blobs(&self) -> Result<BlobStore> {
        Ok(self.ledger()?.blobs.clone())
    }

    pub fn status(&self) -> Result<Status> {
        let ledger = self.ledger()?;
        Ok(Status {
            version: env!("CARGO_PKG_VERSION"),
            records: ledger.log.len(),
            artifacts: ledger.blobs.count()?,
            root: ledger.log.root_hex(),
            signer: self.signer(),
            mode: "local",
        })
    }

    pub fn records(&self, filter: &RecordFilter) -> Result<RecordPage> {
        let ledger = self.ledger()?;
        let limit = filter.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let mut total = 0;
        let mut records = Vec::new();
        for (position, id, signed) in ledger.log.iter() {
            if !filter.matches(&signed.record) {
                continue;
            }
            if total >= filter.offset && records.len() < limit {
                records.push(Summary::new(position, id, signed));
            }
            total += 1;
        }
        Ok(RecordPage {
            records,
            total,
            offset: filter.offset,
            root: ledger.log.root_hex(),
        })
    }

    pub fn record(&self, id: &RecordId) -> Result<RecordDetail> {
        let ledger = self.ledger()?;
        let (position, signed) = lookup(&ledger, id)?;
        let canonical =
            String::from_utf8(signed.canonical_bytes()?).map_err(NodeError::internal)?;
        let artifacts = signed
            .record
            .blobs
            .iter()
            .map(|reference| ArtifactStatus {
                available: ledger.blobs.contains(&reference.blob),
                reference: reference.clone(),
            })
            .collect();
        Ok(RecordDetail {
            id: *id,
            position,
            signed: signed.clone(),
            canonical,
            artifacts,
            children: ledger.log.children_of(id),
        })
    }

    pub fn tree(&self, id: &RecordId) -> Result<Tree> {
        let ledger = self.ledger()?;
        let (position, signed) = lookup(&ledger, id)?;
        let mut missing = Vec::new();
        let ancestors = summaries(&ledger, ledger.log.ancestors_of(id), &mut missing);
        let descendants = summaries(&ledger, ledger.log.descendants_of(id), &mut missing);
        Ok(Tree {
            record: Summary::new(position, *id, signed),
            ancestors,
            descendants,
            missing,
        })
    }

    /// Accept a record signed elsewhere. Re-publishing an existing record is a no-op.
    pub fn append(&self, signed: SignedRecord) -> Result<Published> {
        let mut ledger = self.ledger()?;
        let id = ledger.append(signed)?;
        Published::new(&ledger, id)
    }

    /// Sign a draft with the node's key and append it.
    pub fn publish(&self, draft: Draft) -> Result<Published> {
        let keys = self.keys.as_ref().ok_or(NodeError::SigningDisabled)?;
        let mut record = Record::new(draft.kind, Value::Object(draft.payload), keys.signer())
            .with_parents(draft.parents);
        for blob in draft.blobs {
            record = record.with_blob(blob);
        }
        if let Some(created) = draft.created {
            record = record.with_created(created);
        }
        self.append(record.sign(keys)?)
    }

    pub fn proof(&self, id: &RecordId) -> Result<Proof> {
        let ledger = self.ledger()?;
        let proof = ledger.log.inclusion_proof(id)?;
        proof.verify()?;
        Ok(Proof {
            record: *id,
            leaf_index: proof.leaf_index,
            tree_size: proof.tree_size,
            leaf_hash: hex::encode(proof.leaf_hash),
            path: proof.path.iter().map(hex::encode).collect(),
            root: hex::encode(proof.root),
            valid: true,
            scope: "local tree",
        })
    }

    pub fn verify(&self, id: &RecordId) -> Result<VerifyReport> {
        let ledger = self.ledger()?;
        let (_, signed) = lookup(&ledger, id)?;
        signed.verify()?;
        let audit = ledger.verify_record_blobs(&signed.record)?;
        let proof = ledger.log.inclusion_proof(id)?;
        proof.verify()?;
        Ok(VerifyReport {
            signature: true,
            inclusion: true,
            complete: audit.is_complete(),
            consistent: audit.is_consistent(),
            verified: audit.verified,
            missing: audit.missing,
            corrupt: audit.corrupt,
            verified_bytes: audit.verified_bytes,
            root: hex::encode(proof.root),
            tree_size: proof.tree_size,
        })
    }

    fn ledger(&self) -> Result<MutexGuard<'_, Ledger>> {
        self.ledger
            .lock()
            .map_err(|_| NodeError::internal("ledger lock poisoned"))
    }
}

fn lookup<'a>(ledger: &'a Ledger, id: &RecordId) -> Result<(usize, &'a SignedRecord)> {
    match (ledger.log.position_of(id), ledger.log.get(id)) {
        (Some(position), Some(signed)) => Ok((position, signed)),
        _ => Err(proofprint_core::Error::RecordNotFound(id.to_string()).into()),
    }
}

fn summaries(ledger: &Ledger, ids: Vec<RecordId>, missing: &mut Vec<RecordId>) -> Vec<Summary> {
    ids.into_iter()
        .filter_map(|id| match lookup(ledger, &id) {
            Ok((position, signed)) => Some(Summary::new(position, id, signed)),
            Err(_) => {
                missing.push(id);
                None
            }
        })
        .collect()
}

/// Query parameters of `GET /api/records`.
#[derive(Debug, Default, Deserialize)]
pub struct RecordFilter {
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
    pub kind: Option<String>,
    pub signer: Option<String>,
}

impl RecordFilter {
    fn matches(&self, record: &Record) -> bool {
        self.kind.as_deref().is_none_or(|kind| kind == record.kind)
            && self
                .signer
                .as_deref()
                .is_none_or(|signer| signer == record.signer)
    }
}

/// Body of `POST /api/publish`: an unsigned record the node signs as itself.
#[derive(Debug, Deserialize)]
pub struct Draft {
    pub kind: String,
    #[serde(default)]
    pub payload: Map<String, Value>,
    #[serde(default)]
    pub parents: Vec<RecordId>,
    #[serde(default)]
    pub blobs: Vec<BlobRef>,
    #[serde(default)]
    pub created: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub version: &'static str,
    pub records: usize,
    pub artifacts: usize,
    pub root: Option<String>,
    pub signer: Option<String>,
    pub mode: &'static str,
}

/// One row of a listing.
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub id: RecordId,
    pub position: usize,
    pub kind: String,
    pub created: String,
    pub signer: String,
    pub parents: Vec<RecordId>,
    pub artifacts: usize,
    /// A short name for display. See [`Summary::label`].
    pub label: String,
}

/// Payload fields that name a record, most preferred first. Any payload can
/// use `title`; the others come from the bundled schemas.
const LABEL_FIELDS: &[&str] = &["title", "name", "run_id", "benchmark", "method"];

impl Summary {
    fn new(position: usize, id: RecordId, signed: &SignedRecord) -> Self {
        let record = &signed.record;
        Self {
            id,
            position,
            kind: record.kind.clone(),
            created: record.created.to_rfc3339_opts(SecondsFormat::Secs, true),
            signer: record.signer.clone(),
            parents: record.parents.clone(),
            artifacts: record.blobs.len(),
            label: Self::label(record),
        }
    }

    /// The first string among [`LABEL_FIELDS`], else `step N`, else the kind.
    fn label(record: &Record) -> String {
        let payload = &record.payload;
        if let Some(text) = LABEL_FIELDS
            .iter()
            .find_map(|key| payload.get(key).and_then(Value::as_str))
        {
            return text.to_owned();
        }
        match payload.get("step").and_then(Value::as_u64) {
            Some(step) => format!("step {step}"),
            None => record.kind.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RecordPage {
    pub records: Vec<Summary>,
    /// Records matching the filter, before paging.
    pub total: usize,
    pub offset: usize,
    pub root: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactStatus {
    pub reference: BlobRef,
    /// Bytes are on this node's disk. Says nothing about whether they hash correctly.
    pub available: bool,
}

#[derive(Debug, Serialize)]
pub struct RecordDetail {
    pub id: RecordId,
    pub position: usize,
    pub signed: SignedRecord,
    /// The exact bytes the signature covers. Clients that parse JSON into
    /// doubles should copy this rather than re-serialize `signed.record`.
    pub canonical: String,
    pub artifacts: Vec<ArtifactStatus>,
    pub children: Vec<RecordId>,
}

#[derive(Debug, Serialize)]
pub struct Tree {
    pub record: Summary,
    pub ancestors: Vec<Summary>,
    pub descendants: Vec<Summary>,
    /// Parents named by some record but not present in this log.
    pub missing: Vec<RecordId>,
}

#[derive(Debug, Serialize)]
pub struct Published {
    pub id: RecordId,
    pub position: usize,
    pub root: String,
}

impl Published {
    fn new(ledger: &Ledger, id: RecordId) -> Result<Self> {
        let position = ledger.log.position_of(&id);
        let root = ledger.log.root_hex();
        match (position, root) {
            (Some(position), Some(root)) => Ok(Self { id, position, root }),
            _ => Err(NodeError::internal("appended record is not in the log")),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Proof {
    pub record: RecordId,
    pub leaf_index: u64,
    pub tree_size: u64,
    pub leaf_hash: String,
    pub path: Vec<String>,
    pub root: String,
    pub valid: bool,
    /// The root is this node's own; nobody else has witnessed it.
    pub scope: &'static str,
}

#[derive(Debug, Serialize)]
pub struct VerifyReport {
    pub signature: bool,
    pub inclusion: bool,
    /// Every attachment was present and hashed correctly.
    pub complete: bool,
    /// No present attachment was corrupt. Missing ones do not count against this.
    pub consistent: bool,
    pub verified: Vec<String>,
    pub missing: Vec<String>,
    pub corrupt: Vec<(String, String)>,
    pub verified_bytes: u64,
    pub root: String,
    pub tree_size: u64,
}
