//! Continuity checks over a run's chain of `ml.training-segment/v1` records.
//!
//! This module covers the findings in `spec/v1/training.md` that can be decided
//! from the run and segment payloads alone: step gaps and overlaps, weight
//! discontinuities, forked histories, weight loads, unobserved inputs, and other
//! training in the same process. Findings that need attachments or other
//! records (manifest hashes, model digests, replays) are left to callers that
//! hold a ledger.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::id::RecordId;
use crate::schema::{TrainingRun, TrainingSegment};

/// How serious a finding is.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Note,
    Warning,
    Gap,
}

/// What was found.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    StepsMissing,
    StepsOverlap,
    WeightsChanged,
    TwoHistories,
    WeightsLoaded,
    InputsNotObserved,
    CannotCheck,
    WeightsRestored,
    OtherTraining,
}

impl FindingKind {
    pub fn severity(self) -> Severity {
        use FindingKind::*;
        match self {
            StepsMissing | StepsOverlap | WeightsChanged | TwoHistories => Severity::Gap,
            WeightsLoaded | InputsNotObserved | CannotCheck => Severity::Warning,
            WeightsRestored | OtherTraining => Severity::Note,
        }
    }
}

/// One finding, tied to the segment it concerns when there is one.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<RecordId>,
    pub detail: String,
}

/// A segment and the id of the segment it follows (`None` for the first).
#[derive(Clone, Debug)]
pub struct SegmentEntry {
    pub id: RecordId,
    pub previous: Option<RecordId>,
    pub segment: TrainingSegment,
}

/// The outcome of a check.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Report {
    /// Segments in chain order, as followed from the start of the run.
    pub chain: Vec<RecordId>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// True when no finding is a gap.
    pub fn is_continuous(&self) -> bool {
        self.findings.iter().all(|f| f.severity < Severity::Gap)
    }

    fn push(&mut self, kind: FindingKind, segment: Option<RecordId>, detail: String) {
        self.findings.push(Finding {
            kind,
            severity: kind.severity(),
            segment,
            detail,
        });
    }
}

/// Check the segment chain of `run`. Segments belonging to other runs are ignored.
///
/// Where the history forks, the chain follows the first successor by record id
/// and reports [`FindingKind::TwoHistories`].
pub fn check(run: &TrainingRun, segments: &[SegmentEntry]) -> Report {
    let mut report = Report::default();
    let segments: Vec<&SegmentEntry> = segments
        .iter()
        .filter(|e| e.segment.run_id == run.run_id)
        .collect();

    let mut successors: BTreeMap<Option<RecordId>, Vec<&SegmentEntry>> = BTreeMap::new();
    for entry in &segments {
        successors.entry(entry.previous).or_default().push(entry);
    }
    for (point, next) in successors.iter_mut() {
        next.sort_by_key(|e| e.id);
        if next.len() > 1 {
            let where_ = match point {
                Some(id) => format!("segment {id}"),
                None => "the start of the run".into(),
            };
            report.push(
                FindingKind::TwoHistories,
                *point,
                format!("{} segments follow {where_}", next.len()),
            );
        }
    }

    if run.initial_weights.is_none() {
        report.push(
            FindingKind::CannotCheck,
            None,
            "run does not state initial_weights".into(),
        );
    }

    let mut seen_states: BTreeSet<&str> = BTreeSet::new();
    if let Some(initial) = run.initial_weights.as_deref() {
        seen_states.insert(initial);
    }

    let mut visited = BTreeSet::new();
    let mut previous: Option<&SegmentEntry> = None;
    let mut cursor = None;
    while let Some(entry) = successors.get(&cursor).and_then(|n| n.first()).copied() {
        if !visited.insert(entry.id) {
            break;
        }
        report.chain.push(entry.id);
        check_link(&mut report, run, previous, entry, &seen_states);
        seen_states.insert(entry.segment.weights_before.as_str());
        seen_states.insert(entry.segment.weights_after.as_str());
        previous = Some(entry);
        cursor = Some(entry.id);
    }

    let known: BTreeSet<RecordId> = segments.iter().map(|e| e.id).collect();
    let orphans = segments
        .iter()
        .filter(|e| e.previous.is_some_and(|p| !known.contains(&p)))
        .count();
    if orphans > 0 {
        report.push(
            FindingKind::CannotCheck,
            None,
            format!("{orphans} segment(s) are not reachable from the start of the run"),
        );
    }

    report
}

fn check_link(
    report: &mut Report,
    run: &TrainingRun,
    previous: Option<&SegmentEntry>,
    entry: &SegmentEntry,
    seen_states: &BTreeSet<&str>,
) {
    let seg = &entry.segment;
    let id = Some(entry.id);

    if seg.steps_after < seg.steps_before {
        report.push(
            FindingKind::CannotCheck,
            id,
            format!(
                "steps_after {} is before steps_before {}",
                seg.steps_after, seg.steps_before
            ),
        );
    }

    let expected_steps = previous.map_or(0, |p| p.segment.steps_after);
    if seg.steps_before > expected_steps {
        report.push(
            FindingKind::StepsMissing,
            id,
            format!(
                "starts at step {} but {expected_steps} were recorded",
                seg.steps_before
            ),
        );
    } else if seg.steps_before < expected_steps {
        report.push(
            FindingKind::StepsOverlap,
            id,
            format!(
                "starts at step {} but {expected_steps} were recorded",
                seg.steps_before
            ),
        );
    }

    let expected_weights = match previous {
        Some(p) => Some(p.segment.weights_after.as_str()),
        None => run.initial_weights.as_deref(),
    };
    if let Some(expected) = expected_weights {
        if seg.weights_before != expected {
            report.push(
                FindingKind::WeightsChanged,
                id,
                format!(
                    "weights_before {} differs from {expected}",
                    seg.weights_before
                ),
            );
        }
    }

    if seg.weight_loads > 0 {
        if seen_states.contains(seg.weights_after.as_str()) {
            report.push(
                FindingKind::WeightsRestored,
                id,
                format!("{} load(s) restored an earlier state", seg.weight_loads),
            );
        } else {
            report.push(
                FindingKind::WeightsLoaded,
                id,
                format!(
                    "{} load(s) of state not recorded in this run",
                    seg.weight_loads
                ),
            );
        }
    }
    if seg.steps_without_inputs > 0 {
        report.push(
            FindingKind::InputsNotObserved,
            id,
            format!(
                "{} step(s) without observed inputs",
                seg.steps_without_inputs
            ),
        );
    }
    if seg.other_optimizer_steps > 0 {
        report.push(
            FindingKind::OtherTraining,
            id,
            format!(
                "{} optimizer step(s) on another model",
                seg.other_optimizer_steps
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(n: u8) -> RecordId {
        RecordId::from_digest([n; 32])
    }

    fn run() -> TrainingRun {
        TrainingRun {
            run_id: "r".into(),
            initial_weights: Some("sha256:a".into()),
            ..Default::default()
        }
    }

    fn seg(id: u8, prev: Option<u8>, steps: (u64, u64), w: (&str, &str)) -> SegmentEntry {
        SegmentEntry {
            id: rid(id),
            previous: prev.map(rid),
            segment: TrainingSegment {
                run_id: "r".into(),
                steps_before: steps.0,
                steps_after: steps.1,
                weights_before: w.0.into(),
                weights_after: w.1.into(),
                ..Default::default()
            },
        }
    }

    fn kinds(report: &Report) -> Vec<FindingKind> {
        report.findings.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn clean_chain_is_continuous() {
        let report = check(
            &run(),
            &[
                seg(2, Some(1), (10, 20), ("sha256:b", "sha256:c")),
                seg(1, None, (0, 10), ("sha256:a", "sha256:b")),
            ],
        );
        assert!(report.is_continuous(), "{:?}", report.findings);
        assert_eq!(report.chain, vec![rid(1), rid(2)]);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn gaps_are_reported() {
        let report = check(
            &run(),
            &[
                seg(1, None, (5, 10), ("sha256:x", "sha256:b")),
                seg(2, Some(1), (8, 20), ("sha256:b", "sha256:c")),
                seg(3, Some(2), (25, 30), ("sha256:z", "sha256:d")),
            ],
        );
        assert!(!report.is_continuous());
        assert_eq!(
            kinds(&report),
            vec![
                FindingKind::StepsMissing,
                FindingKind::WeightsChanged,
                FindingKind::StepsOverlap,
                FindingKind::StepsMissing,
                FindingKind::WeightsChanged,
            ]
        );
    }

    #[test]
    fn forks_are_reported() {
        let report = check(
            &run(),
            &[
                seg(1, None, (0, 10), ("sha256:a", "sha256:b")),
                seg(2, Some(1), (10, 20), ("sha256:b", "sha256:c")),
                seg(3, Some(1), (10, 20), ("sha256:b", "sha256:d")),
            ],
        );
        assert_eq!(kinds(&report), vec![FindingKind::TwoHistories]);
        assert_eq!(report.chain, vec![rid(1), rid(2)]);
    }

    #[test]
    fn loads_and_observation_counts() {
        let mut restored = seg(1, None, (0, 10), ("sha256:a", "sha256:a"));
        restored.segment.weight_loads = 1;
        restored.segment.steps_without_inputs = 2;
        let mut loaded = seg(2, Some(1), (10, 20), ("sha256:a", "sha256:q"));
        loaded.segment.weight_loads = 1;
        loaded.segment.other_optimizer_steps = 3;
        let report = check(&run(), &[restored, loaded]);
        assert_eq!(
            kinds(&report),
            vec![
                FindingKind::WeightsRestored,
                FindingKind::InputsNotObserved,
                FindingKind::WeightsLoaded,
                FindingKind::OtherTraining,
            ]
        );
        assert!(report.is_continuous());
    }

    #[test]
    fn missing_initial_weights_cannot_be_checked() {
        let mut run = run();
        run.initial_weights = None;
        let report = check(&run, &[seg(1, None, (0, 1), ("sha256:a", "sha256:b"))]);
        assert_eq!(kinds(&report), vec![FindingKind::CannotCheck]);
    }

    #[test]
    fn other_runs_are_ignored() {
        let mut other = seg(9, None, (3, 4), ("x", "y"));
        other.segment.run_id = "other".into();
        let report = check(&run(), &[other]);
        assert!(report.chain.is_empty());
        assert!(report.findings.is_empty());
    }
}
