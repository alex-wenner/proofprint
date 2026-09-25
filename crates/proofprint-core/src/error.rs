//! Errors for [`proofprint_core`].

use std::path::PathBuf;

/// Anything that can go wrong while creating, storing, or verifying a record.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ledger is already open by another process: {0}")]
    LedgerInUse(PathBuf),
    #[error("invalid content id {0:?}")]
    InvalidContentId(String),

    #[error("invalid record id {0:?}")]
    InvalidRecordId(String),

    #[error("invalid signer {0:?}")]
    InvalidSigner(String),

    #[error("malformed record: {0}")]
    MalformedRecord(String),

    #[error("invalid key material: {0}")]
    InvalidKey(String),

    #[error("key file not found: {0}")]
    KeyNotFound(PathBuf),

    #[error("blob not found: {0}")]
    BlobNotFound(String),

    #[error("blob {expected} hashes to {actual}")]
    BlobIntegrity { expected: String, actual: String },

    #[error("record not found: {0}")]
    RecordNotFound(String),

    #[error("parent record not found in log: {0}")]
    ParentNotFound(String),

    #[error("signature verification failed")]
    BadSignature,

    #[error("unsupported record version {0} (expected {1})")]
    UnsupportedVersion(u8, u8),

    #[error("invalid inclusion proof")]
    BadInclusionProof,

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
