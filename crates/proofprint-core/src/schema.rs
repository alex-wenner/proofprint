//! Well-known record kinds and the schema extension pattern.
//!
//! The ledger core is domain-neutral: it stores and orders signed records
//! without interpreting them. Meaning lives in schemas, identified by the
//! `kind` string (`name/vN`).
//!
//! To add a domain, define a payload struct and implement [`Schema`]:
//!
//! ```
//! use proofprint_core::schema::Schema;
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct MyEvent { thing: String, count: u64 }
//!
//! impl Schema for MyEvent {
//!     const KIND: &'static str = "example.my-event/v1";
//! }
//!
//! let record = MyEvent { thing: "x".into(), count: 3 }
//!     .to_record("ed25519:0000000000000000000000000000000000000000000000000000000000000000")
//!     .unwrap();
//! assert_eq!(record.kind, "example.my-event/v1");
//! ```
//!
//! # Versioning
//!
//! Bump `/vN` for breaking changes. A verifier that does not understand a kind
//! can still check the signature, the ordering, and the DAG — it just cannot
//! interpret the payload. That is the property that lets this stay general.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::record::Record;

/// A typed payload that knows its `kind`.
pub trait Schema: Serialize {
    /// The `name/vN` identifier for this payload shape.
    const KIND: &'static str;

    /// Wrap this payload in an unsigned [`Record`].
    fn to_record(self, signer: impl Into<String>) -> Result<Record>
    where
        Self: Sized,
    {
        Ok(Record::new(
            Self::KIND,
            serde_json::to_value(&self)?,
            signer,
        ))
    }
}

/// Decode a record's payload into a typed schema, checking that `kind` matches.
///
/// This is how an auditor consumes records: parse to a known shape and reject
/// anything that does not fit.
pub fn decode<S: Schema + DeserializeOwned>(record: &Record) -> Result<S> {
    if record.kind != S::KIND {
        return Err(crate::error::Error::MalformedRecord(format!(
            "record kind {} is not {}",
            record.kind,
            S::KIND
        )));
    }
    Ok(serde_json::from_value(record.payload.clone())?)
}

// ---------------------------------------------------------------------------
// Machine-learning schemas (the first domain)
// ---------------------------------------------------------------------------

/// `ml.dataset/v1` — a dataset and how it was produced.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Dataset {
    /// Human-readable name.
    pub name: String,
    /// Version tag within that name.
    pub version: String,
    /// Where it came from: crawl, upload, synthesis, derivation, ...
    pub origin: String,
    /// Declared license or terms, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Number of examples, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples: Option<u64>,
    /// Manifest blob name holding the sample list or its Merkle root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_blob: Option<String>,
    /// Free-form notes.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub notes: Value,
}

impl Schema for Dataset {
    const KIND: &'static str = "ml.dataset/v1";
}

/// `ml.model/v1` — a model artifact and its architecture.
#[derive(Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Model {
    /// Human-readable name.
    pub name: String,
    /// Version tag.
    pub version: String,
    /// Architecture family, e.g. `transformer-decoder`.
    pub architecture: String,
    /// Parameter count, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<u64>,
    /// Blob name holding the weights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights_blob: Option<String>,
    /// Executable architecture definition blob name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_blob: Option<String>,
    /// Manifest digest of the weights, as in `spec/v1/training.md`. The
    /// manifest itself is attached as `weights-manifest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights_digest: Option<String>,
    /// Free-form notes.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub notes: Value,
}

impl Schema for Model {
    const KIND: &'static str = "ml.model/v1";
}

/// `ml.training-run/v1` — the declared recipe for a training run.
///
/// This is a *claim about intent*: which data, which code, which
/// hyperparameters, which starting state. Verifying that the run actually
/// followed it is a separate audit.
#[derive(Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TrainingRun {
    /// Run identifier chosen by the publisher.
    pub run_id: String,
    /// Framework and version, e.g. `pytorch/2.5.1`.
    pub framework: String,
    /// Source control commit of the training code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_commit: Option<String>,
    /// Hyperparameters and schedule.
    pub config: Value,
    /// Declared hardware environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Value>,
    /// Total planned steps, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_steps: Option<u64>,
    /// Manifest digest of the weights when recording started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_weights: Option<String>,
    /// Hashes of the source files that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeManifest>,
    /// Name and version of the software that observed the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorder: Option<String>,
}

impl Schema for TrainingRun {
    const KIND: &'static str = "ml.training-run/v1";
}

/// The entry script and the project's own modules, by path, with `sha256:` digests.
#[derive(Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeManifest {
    pub entry: String,
    pub files: std::collections::BTreeMap<String, String>,
}

/// `ml.training-segment/v1` — a stretch of training between two recorded states.
///
/// Parents are the run and the previous segment. The chain of segments is what
/// [`crate::continuity`] checks; field meanings are in `spec/v1/training.md`.
#[derive(Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TrainingSegment {
    /// The run's `run_id`.
    pub run_id: String,
    /// Optimizer steps on the model since the run began, before this segment.
    pub steps_before: u64,
    /// Optimizer steps on the model since the run began, after this segment.
    pub steps_after: u64,
    /// Manifest digest at the start of the segment.
    pub weights_before: String,
    /// Manifest digest at the end of the segment.
    pub weights_after: String,
    /// Digest over the per-step input digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<String>,
    /// Steps with no training-mode forward call observed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub steps_without_inputs: u64,
    /// Optimizer steps in the same process that updated a different model.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub other_optimizer_steps: u64,
    /// Times the model's state was loaded from elsewhere during the segment.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub weight_loads: u64,
    /// Wall-clock seconds the segment covered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// Hardware the model was on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl Schema for TrainingSegment {
    const KIND: &'static str = "ml.training-segment/v1";
}

fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// `ml.training-step/v1` — one committed point in a training trajectory.
///
/// Parents should be the run record and the previous step, forming a hash chain
/// over the trajectory.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TrainingStep {
    /// Zero-based step counter.
    pub step: u64,
    /// Number of examples consumed so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples_seen: Option<u64>,
    /// Reported loss at this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<f64>,
    /// Additional reported metrics.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metrics: Value,
    /// Blob name of the checkpoint committed at this step, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_blob: Option<String>,
}

impl Schema for TrainingStep {
    const KIND: &'static str = "ml.training-step/v1";
}

/// `ml.evaluation/v1` — a measured result against a named harness.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Evaluation {
    /// Harness name and version, e.g. `lm-eval-harness/0.4`.
    pub harness: String,
    /// Benchmark name.
    pub benchmark: String,
    /// Measured scores keyed by metric name.
    pub scores: Value,
    /// Blob name holding the full result dump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_blob: Option<String>,
}

impl Schema for Evaluation {
    const KIND: &'static str = "ml.evaluation/v1";
}

/// `attestation.verification/v1` — an independent party's audit result.
///
/// This is how third-party findings attach to someone else's records: the
/// verifier signs its own record whose parent is the record under audit.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Verification {
    /// What was checked, e.g. `signature`, `checkpoint-replay`, `contamination`.
    pub method: String,
    /// Outcome.
    pub outcome: VerificationOutcome,
    /// Human-readable explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Blob name holding evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_blob: Option<String>,
}

/// Result of an audit.
#[derive(Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    /// The check passed.
    Pass,
    /// The check failed.
    Fail,
    /// The check could not be completed.
    Inconclusive,
}

impl Schema for Verification {
    const KIND: &'static str = "attestation.verification/v1";
}

/// Every kind defined by this crate, for documentation and tooling.
pub const KNOWN_KINDS: &[&str] = &[
    Dataset::KIND,
    Model::KIND,
    TrainingRun::KIND,
    TrainingStep::KIND,
    TrainingSegment::KIND,
    Evaluation::KIND,
    Verification::KIND,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyPair;
    use serde_json::json;

    #[test]
    fn to_record_sets_kind_and_signer() {
        let kp = KeyPair::generate();
        let rec = Dataset {
            name: "web-crawl".into(),
            version: "2026-01".into(),
            origin: "crawl".into(),
            license: Some("CC-BY-4.0".into()),
            examples: Some(1_000),
            manifest_blob: None,
            notes: Value::Null,
        }
        .to_record(kp.signer())
        .unwrap();

        assert_eq!(rec.kind, "ml.dataset/v1");
        assert_eq!(rec.signer, kp.signer());
        assert!(rec.validate().is_ok());
    }

    #[test]
    fn decode_round_trips() {
        let kp = KeyPair::generate();
        let step = TrainingStep {
            step: 42,
            samples_seen: Some(4200),
            loss: Some(2.5),
            metrics: json!({"grad_norm": 1.1}),
            checkpoint_blob: None,
        };
        let rec = step.clone().to_record(kp.signer()).unwrap();
        let back: TrainingStep = decode(&rec).unwrap();
        assert_eq!(back, step);
    }

    #[test]
    fn decode_rejects_wrong_kind() {
        let kp = KeyPair::generate();
        let rec = TrainingStep {
            step: 1,
            samples_seen: None,
            loss: None,
            metrics: Value::Null,
            checkpoint_blob: None,
        }
        .to_record(kp.signer())
        .unwrap();
        assert!(decode::<Dataset>(&rec).is_err());
    }

    #[test]
    fn decode_rejects_mismatched_payload() {
        let kp = KeyPair::generate();
        let rec = Record::new("ml.dataset/v1", json!({"unrelated": true}), kp.signer());
        assert!(decode::<Dataset>(&rec).is_err());
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let kp = KeyPair::generate();
        let rec = TrainingStep {
            step: 1,
            samples_seen: None,
            loss: None,
            metrics: Value::Null,
            checkpoint_blob: None,
        }
        .to_record(kp.signer())
        .unwrap();
        let s = serde_json::to_string(&rec.payload).unwrap();
        assert!(!s.contains("samples_seen"), "{s}");
        assert!(!s.contains("metrics"), "{s}");
        assert!(s.contains("\"step\":1"), "{s}");
    }

    #[test]
    fn known_kinds_have_valid_grammar() {
        use crate::record::utc;
        let kp = KeyPair::generate();
        for k in KNOWN_KINDS {
            let rec = Record::new(*k, json!({}), kp.signer()).with_created(utc(0));
            assert!(rec.validate().is_ok(), "{k} should be accepted");
        }
    }

    #[test]
    fn verification_record_signs_and_decodes() {
        let kp = KeyPair::generate();
        let v = Verification {
            method: "signature".into(),
            outcome: VerificationOutcome::Pass,
            detail: Some("ed25519 signature valid".into()),
            evidence_blob: None,
        };
        let rec = v.clone().to_record(kp.signer()).unwrap();
        let signed = rec.sign(&kp).unwrap();
        assert!(signed.verify().is_ok());
        let back: Verification = decode(&signed.record).unwrap();
        assert_eq!(back.outcome, VerificationOutcome::Pass);
    }
}
