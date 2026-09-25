//! Labeled example records for a first look. Nothing here describes real
//! training, and every payload says so.

use proofprint_core::schema::{
    Dataset, Evaluation, Model, Schema, TrainingRun, TrainingStep, Verification,
    VerificationOutcome,
};
use proofprint_core::{BlobRef, KeyPair, Ledger, RecordId};
use serde_json::json;

const MANIFEST: &[u8] =
    b"demonstration line one\ndemonstration line two\ndemonstration line three\n";
const WEIGHTS: &[u8] = b"demonstration weights; not a model\n";

/// Fill an empty ledger with a small training history from two signers.
///
/// Returns the ids in append order. An empty vector means the ledger already
/// had records and nothing was written.
pub fn seed(ledger: &mut Ledger, publisher: &KeyPair) -> proofprint_core::Result<Vec<RecordId>> {
    if !ledger.log.is_empty() {
        return Ok(Vec::new());
    }
    let reviewer = KeyPair::generate();
    let manifest = ledger.attach_bytes("manifest", MANIFEST, Some("text/plain".into()))?;
    let weights = ledger.attach_bytes("weights", WEIGHTS, Some("text/plain".into()))?;
    let mut seeder = Seeder {
        ledger,
        ids: Vec::new(),
    };

    let dataset = seeder.publish(
        publisher,
        Dataset {
            name: "demo-corpus".into(),
            version: "2026.01".into(),
            origin: "synthetic".into(),
            license: Some("CC0-1.0".into()),
            examples: Some(3),
            manifest_blob: Some("manifest".into()),
            notes: json!({
                "demonstration": true,
                "description": "Three placeholder lines. Nobody trained on this."
            }),
        },
        &[],
        vec![manifest],
    )?;
    let run = seeder.publish(
        publisher,
        TrainingRun {
            run_id: "demo-run-1".into(),
            framework: "none/0".into(),
            code_commit: None,
            config: json!({
                "demonstration": true,
                "optimizer": "adamw",
                "learning_rate": 0.0003,
                "steps": 1200
            }),
            environment: None,
            total_steps: Some(1200),
            ..Default::default()
        },
        &[dataset],
        vec![],
    )?;
    let step_600 = seeder.publish(
        publisher,
        TrainingStep {
            step: 600,
            samples_seen: Some(1800),
            loss: Some(2.81),
            metrics: json!({ "demonstration": true }),
            checkpoint_blob: None,
        },
        &[run],
        vec![],
    )?;
    let step_1200 = seeder.publish(
        publisher,
        TrainingStep {
            step: 1200,
            samples_seen: Some(3600),
            loss: Some(2.34),
            metrics: json!({ "demonstration": true }),
            checkpoint_blob: Some("weights".into()),
        },
        &[step_600],
        vec![weights.clone()],
    )?;
    let model = seeder.publish(
        publisher,
        Model {
            name: "demo-model".into(),
            version: "0.1.0-demo".into(),
            architecture: "none".into(),
            parameters: Some(0),
            weights_blob: Some("weights".into()),
            definition_blob: None,
            notes: json!({
                "demonstration": true,
                "description": "The weights attachment is a short text file."
            }),
            ..Default::default()
        },
        &[step_1200],
        vec![weights],
    )?;
    let evaluation = seeder.publish(
        publisher,
        Evaluation {
            harness: "none/0".into(),
            benchmark: "demo-benchmark".into(),
            scores: json!({ "demonstration": true, "accuracy": 0.0 }),
            report_blob: None,
        },
        &[model],
        vec![],
    )?;
    let re_run = seeder.publish(
        &reviewer,
        Evaluation {
            harness: "none/0".into(),
            benchmark: "demo-benchmark".into(),
            scores: json!({
                "demonstration": true,
                "accuracy": 0.0,
                "note": "published by a second key to stand in for an outside re-run"
            }),
            report_blob: None,
        },
        &[model],
        vec![],
    )?;
    seeder.publish(
        &reviewer,
        Verification {
            method: "re-evaluation".into(),
            outcome: VerificationOutcome::Inconclusive,
            detail: Some(
                "Demonstration only. Signed by a second key to show a record from another party; nothing was measured."
                    .into(),
            ),
            evidence_blob: None,
        },
        &[evaluation, re_run],
        vec![],
    )?;
    Ok(seeder.ids)
}

struct Seeder<'a> {
    ledger: &'a mut Ledger,
    ids: Vec<RecordId>,
}

impl Seeder<'_> {
    fn publish<S: Schema>(
        &mut self,
        keys: &KeyPair,
        payload: S,
        parents: &[RecordId],
        blobs: Vec<BlobRef>,
    ) -> proofprint_core::Result<RecordId> {
        let mut record = payload
            .to_record(keys.signer())?
            .with_parents(parents.iter().copied());
        for blob in blobs {
            record = record.with_blob(blob);
        }
        let id = self.ledger.publish(record, keys)?;
        self.ids.push(id);
        Ok(id)
    }
}
