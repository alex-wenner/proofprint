//! The record model: what gets signed, hashed, and appended to the log.
//!
//! A [`Record`] is deliberately **domain-neutral**. The core ledger never
//! interprets `payload`; it only guarantees that the bytes were committed,
//! signed and ordered. Independent retention of commitments exposes later
//! changes. Domain meaning comes from
//! `kind`, which names a schema (see `spec/v1/record.md`).
//!
//! ```text
//! Record                       the unsigned, canonicalized, content-addressed claim
//!   ├── v          protocol version
//!   ├── kind       schema reference, e.g. "ml.training-step/v1"
//!   ├── parents    DAG edges to other records
//!   ├── payload    domain-specific JSON object
//!   ├── blobs      content-addressed attachments
//!   ├── created    RFC 3339 UTC, seconds precision
//!   └── signer     "ed25519:<hex>"
//!
//! SignedRecord                 { record, sig }
//! ```
//!
//! The record id is the [`RecordId`] of the canonical encoding of the *unsigned*
//! record, so an id is stable regardless of signature or serialization order.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::canonical;
use crate::error::{Error, Result};
use crate::id::{BlobId, RecordId};
use crate::key::{KeyPair, PublicKey};

/// Protocol version emitted and accepted by this build.
pub const PROTOCOL_VERSION: u8 = 1;

/// Maximum canonical payload size accepted inline. Larger data belongs in a blob.
pub const MAX_INLINE_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Maximum number of parent references.
pub const MAX_PARENTS: usize = 64;
/// Maximum number of blob references.
pub const MAX_BLOBS: usize = 64;
/// Maximum length of a `kind` string.
pub const MAX_KIND_LEN: usize = 128;
/// Maximum length of a blob reference name.
pub const MAX_BLOB_NAME_LEN: usize = 256;
/// Maximum clock skew tolerated for `created`.
pub const MAX_FUTURE_SKEW_SECS: i64 = 300;

/// A content-addressed attachment referenced by a record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BlobRef {
    /// Human-readable role of this attachment, unique within a record.
    pub name: String,
    /// Content id of the bytes.
    pub blob: BlobId,
    /// Declared size in bytes.
    pub size: u64,
    /// Optional media type, e.g. `application/octet-stream`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}

impl BlobRef {
    /// Build a reference from raw bytes, computing the id and size.
    pub fn from_bytes(name: impl Into<String>, bytes: &[u8], media: Option<String>) -> Self {
        Self {
            name: name.into(),
            blob: BlobId::from_bytes(bytes),
            size: bytes.len() as u64,
            media,
        }
    }
}

/// An unsigned audit record.
///
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Record {
    /// Protocol version.
    pub v: u8,
    /// Schema reference in the form `name/vN`.
    pub kind: String,
    /// Parent record IDs, forming the artifact's dependency graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<RecordId>,
    /// Domain-specific data, interpreted according to `kind`.
    pub payload: Value,
    /// Content-addressed attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<BlobRef>,
    /// Creation time, RFC 3339 UTC at seconds precision.
    #[serde(with = "rfc3339_secs")]
    pub created: DateTime<Utc>,
    /// Signer public key, `ed25519:<hex>`.
    pub signer: String,
}

impl Record {
    /// Start a record with `payload` and `signer`, timestamped now.
    pub fn new(kind: impl Into<String>, payload: Value, signer: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            kind: kind.into(),
            parents: Vec::new(),
            payload,
            blobs: Vec::new(),
            created: utc(Utc::now().timestamp()),
            signer: signer.into(),
        }
    }

    /// Add a parent reference.
    pub fn with_parent(mut self, parent: RecordId) -> Self {
        self.parents.push(parent);
        self
    }

    /// Add several parent references.
    pub fn with_parents<I: IntoIterator<Item = RecordId>>(mut self, parents: I) -> Self {
        self.parents.extend(parents);
        self
    }

    /// Add a blob reference.
    pub fn with_blob(mut self, blob: BlobRef) -> Self {
        self.blobs.push(blob);
        self
    }

    /// Override the creation time (useful for deterministic tests).
    pub fn with_created(mut self, created: DateTime<Utc>) -> Self {
        self.created = utc(created.timestamp());
        self
    }

    /// The canonical bytes that get signed and hashed.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical::encode(self)
    }

    /// The content-addressed record id.
    pub fn id(&self) -> Result<RecordId> {
        Ok(RecordId::from_bytes(&self.canonical_bytes()?))
    }

    /// Enforce structural rules. Called before signing and before accepting a
    /// record from the network.
    pub fn validate(&self) -> Result<()> {
        if self.v != PROTOCOL_VERSION {
            return Err(Error::UnsupportedVersion(self.v, PROTOCOL_VERSION));
        }
        if !is_valid_kind(&self.kind) {
            return Err(Error::MalformedRecord(format!(
                "kind {0:?} must match <name>/v<N>",
                self.kind
            )));
        }
        if self.parents.len() > MAX_PARENTS {
            return Err(Error::MalformedRecord(format!(
                "{} parents exceeds limit {MAX_PARENTS}",
                self.parents.len()
            )));
        }
        if self.blobs.len() > MAX_BLOBS {
            return Err(Error::MalformedRecord(format!(
                "{} blobs exceeds limit {MAX_BLOBS}",
                self.blobs.len()
            )));
        }
        if !matches!(self.payload, Value::Object(_)) {
            return Err(Error::MalformedRecord(
                "payload must be a JSON object".to_string(),
            ));
        }

        let mut seen_parents = self.parents.clone();
        seen_parents.sort_unstable();
        if seen_parents.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::MalformedRecord("duplicate parent reference".into()));
        }

        let mut names: Vec<&str> = self.blobs.iter().map(|b| b.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::MalformedRecord("duplicate blob name".into()));
        }
        for b in &self.blobs {
            if b.name.is_empty() || b.name.len() > MAX_BLOB_NAME_LEN {
                return Err(Error::MalformedRecord(format!(
                    "blob name {0:?} must be 1..={MAX_BLOB_NAME_LEN} chars",
                    b.name
                )));
            }
        }

        // Reject a signer string we cannot parse, so verification can never
        // succeed against a malformed key.
        PublicKey::parse(&self.signer)?;

        let size = self.canonical_bytes()?.len();
        if size > MAX_INLINE_PAYLOAD_BYTES {
            return Err(Error::MalformedRecord(format!(
                "canonical record is {size} bytes, exceeds {MAX_INLINE_PAYLOAD_BYTES}"
            )));
        }

        let skew = self.created.signed_duration_since(Utc::now()).num_seconds();
        if skew > MAX_FUTURE_SKEW_SECS {
            return Err(Error::MalformedRecord(format!(
                "created timestamp is {skew}s in the future (max {MAX_FUTURE_SKEW_SECS})"
            )));
        }

        Ok(())
    }

    /// Sign this record with `keys`, producing a [`SignedRecord`].
    pub fn sign(self, keys: &KeyPair) -> Result<SignedRecord> {
        SignedRecord::sign(self, keys)
    }
}

/// A record plus its Ed25519 signature over the canonical encoding.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SignedRecord {
    /// The signed record.
    pub record: Record,
    /// Hex-encoded Ed25519 signature over `record`'s canonical bytes.
    pub sig: String,
}

impl SignedRecord {
    /// Validate, canonicalize, and sign `record`.
    pub fn sign(record: Record, keys: &KeyPair) -> Result<Self> {
        record.validate()?;
        if record.signer != keys.signer() {
            return Err(Error::MalformedRecord(format!(
                "record signer {} does not match signing key {}",
                record.signer,
                keys.signer()
            )));
        }
        let bytes = record.canonical_bytes()?;
        let sig = keys.sign(&bytes);
        Ok(Self {
            record,
            sig: hex::encode(sig.to_bytes()),
        })
    }

    /// The record id.
    pub fn id(&self) -> Result<RecordId> {
        self.record.id()
    }

    /// The canonical bytes that were signed.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.record.canonical_bytes()
    }

    /// Check structure, signature, and self-consistency. Returns the record id.
    ///
    /// This verifies *authorship and integrity*. It does **not** verify anything
    /// about the physical process the record describes — that is the job of
    /// domain-specific auditors.
    pub fn verify(&self) -> Result<RecordId> {
        self.record.validate()?;

        let sig_bytes = hex::decode(&self.sig)
            .map_err(|_| Error::MalformedRecord("sig is not valid hex".into()))?;
        let signature = crate::key::signature_from_slice(&sig_bytes)?;

        let pubkey = PublicKey::parse(&self.record.signer)?;
        pubkey.verify(&self.canonical_bytes()?, &signature)?;

        self.id()
    }

    /// True if `other` has the same id.
    pub fn same_id_as(&self, other: &SignedRecord) -> bool {
        match (self.id(), other.id()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

/// Validate the `name/vN` kind grammar without pulling in a regex crate.
fn is_valid_kind(kind: &str) -> bool {
    if kind.is_empty() || kind.len() > MAX_KIND_LEN {
        return false;
    }
    let Some((name, version)) = kind.rsplit_once('/') else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    let Some(digits) = version.strip_prefix('v') else {
        return false;
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    name.split('.').all(is_valid_segment)
}

fn is_valid_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !seg.starts_with('-')
        && !seg.ends_with('-')
}

/// Fixed-precision RFC 3339 so timestamps are deterministic across encodings.
mod rfc3339_secs {
    use super::*;

    const FORMAT: SecondsFormat = SecondsFormat::Secs;

    pub fn serialize<S: Serializer>(
        dt: &DateTime<Utc>,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&dt.to_rfc3339_opts(FORMAT, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<DateTime<Utc>, D::Error> {
        let s = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

/// Convenience for building UTC timestamps in tests and fixtures.
pub fn utc(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (KeyPair, Record) {
        let kp = KeyPair::generate();
        let rec = Record::new("ml.training-step/v1", json!({"step": 1}), kp.signer())
            .with_created(utc(1_700_000_000));
        (kp, rec)
    }

    #[test]
    fn kind_grammar() {
        for good in [
            "ml.training-step/v1",
            "software.release/v12",
            "a/v1",
            "sensor.observation-x/v2",
            "x.y.z/v3",
        ] {
            assert!(is_valid_kind(good), "{good} should be valid");
        }
        for bad in [
            "training",
            "training/v",
            "/v1",
            "training/",
            "Training/v1",
            "training/v1.2",
            "-train/v1",
            "train-/v1",
            "train//v1",
            "",
        ] {
            assert!(!is_valid_kind(bad), "{bad} should be rejected");
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let (kp, rec) = fixture();
        let signed = rec.sign(&kp).unwrap();
        let id = signed.verify().unwrap();
        assert_eq!(id, signed.id().unwrap());
    }

    #[test]
    fn record_id_is_stable_across_key_order() {
        let (kp, _) = fixture();
        let a = Record::new("x.y/v1", json!({"a": 1, "b": 2}), kp.signer()).with_created(utc(0));
        let b = Record::new("x.y/v1", json!({"b": 2, "a": 1}), kp.signer()).with_created(utc(0));
        assert_eq!(a.id().unwrap(), b.id().unwrap());
    }

    #[test]
    fn record_id_ignores_signature() {
        let (kp, rec) = fixture();
        let signed = rec.clone().sign(&kp).unwrap();
        assert_eq!(rec.id().unwrap(), signed.id().unwrap());
    }

    #[test]
    fn tampering_invalidates_signature() {
        let (kp, rec) = fixture();
        let mut signed = rec.sign(&kp).unwrap();
        signed.record.payload = json!({"step": 999});
        assert!(matches!(signed.verify(), Err(Error::BadSignature)));
    }

    #[test]
    fn signing_with_mismatched_key_is_rejected() {
        let (_kp, rec) = fixture();
        let other = KeyPair::generate();
        assert!(matches!(rec.sign(&other), Err(Error::MalformedRecord(_))));
    }

    #[test]
    fn validate_rejects_duplicate_parents_and_blobs() {
        let (kp, _) = fixture();
        let p = RecordId::from_bytes(b"p");
        let bad = Record::new("x.y/v1", json!({}), kp.signer())
            .with_parent(p)
            .with_parent(p)
            .with_created(utc(0));
        assert!(matches!(bad.validate(), Err(Error::MalformedRecord(_))));

        let b = BlobRef::from_bytes("same", b"data", None);
        let bad = Record::new("x.y/v1", json!({}), kp.signer())
            .with_blob(b.clone())
            .with_blob(b)
            .with_created(utc(0));
        assert!(matches!(bad.validate(), Err(Error::MalformedRecord(_))));
    }

    #[test]
    fn validate_rejects_non_object_payload() {
        let kp = KeyPair::generate();
        let rec = Record::new("x.y/v1", json!([1, 2]), kp.signer()).with_created(utc(0));
        assert!(matches!(rec.validate(), Err(Error::MalformedRecord(_))));
    }

    #[test]
    fn validate_rejects_future_timestamp() {
        let kp = KeyPair::generate();
        let rec = Record::new("x.y/v1", json!({}), kp.signer())
            .with_created(Utc::now() + chrono::Duration::hours(1));
        assert!(matches!(rec.validate(), Err(Error::MalformedRecord(_))));
    }

    #[test]
    fn json_round_trip_is_lossless() {
        let (kp, rec) = fixture();
        let signed = rec
            .with_parent(RecordId::from_bytes(b"parent"))
            .with_blob(BlobRef::from_bytes(
                "checkpoint",
                b"weights",
                Some("application/octet-stream".into()),
            ))
            .sign(&kp)
            .unwrap();

        let json = serde_json::to_string(&signed).unwrap();
        let back: SignedRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, signed);
        assert!(back.verify().is_ok());
    }

    #[test]
    fn timestamp_serializes_at_seconds_precision() {
        let (kp, rec) = fixture();
        let json = serde_json::to_string(&rec.with_created(utc(0))).unwrap();
        assert!(
            json.contains("\"created\":\"1970-01-01T00:00:00Z\""),
            "{json}"
        );
        let _ = kp;
    }
}
