//! Property test — the same `Proposal<W>` produces the same
//! fingerprint regardless of the Repository adapter.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use knotch_derive::MilestoneKind;
use knotch_kernel::{
    AppendMode, Causation, Fingerprint, PhaseKind, Proposal, Repository, Scope, UnitId,
    WorkflowKind,
    causation::{Source, Trigger},
    event::{CommitKind, CommitRef, CommitStatus, EventBody, SkipKind},
    fingerprint_event, fingerprint_proposal,
};
use knotch_storage::FileRepository;
use knotch_testing::InMemoryRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum Phase {
    Only,
}
impl PhaseKind for Phase {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("only")
    }
    fn is_skippable(&self, _: &SkipKind) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
enum Milestone {
    Alpha,
    Beta,
    Gamma,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Gate {}
impl knotch_kernel::GateKind for Gate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

#[derive(Debug, Clone, Copy)]
struct Wf;
const PHASES: [Phase; 1] = [Phase::Only];
impl WorkflowKind for Wf {
    type Phase = Phase;
    type Milestone = Milestone;
    type Gate = Gate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("parity")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
}

fn proposal(body: EventBody<Wf>) -> Proposal<Wf> {
    Proposal {
        causation: Causation::new(Source::Cli, Trigger::Command { name: "test".into() }),
        extension: (),
        body,
        supersedes: None,
    }
}

fn bodies() -> Vec<EventBody<Wf>> {
    vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::MilestoneShipped {
            milestone: Milestone::Alpha,
            commit: CommitRef::new("abc1234"),
            commit_kind: CommitKind::Feat,
            status: CommitStatus::Verified,
        },
        EventBody::MilestoneShipped {
            milestone: Milestone::Beta,
            commit: CommitRef::new("def5678"),
            commit_kind: CommitKind::Fix,
            status: CommitStatus::Verified,
        },
        EventBody::MilestoneShipped {
            milestone: Milestone::Gamma,
            commit: CommitRef::new("cafe1234"),
            commit_kind: CommitKind::Refactor,
            status: CommitStatus::Pending,
        },
    ]
}

#[test]
fn fingerprint_proposal_is_pure_over_body_shape() {
    // `fingerprint_proposal` depends only on body + supersede target
    // + workflow salt. Different causations must not change the
    // fingerprint (they are metadata, not identity).
    for body in bodies() {
        let base = proposal(body.clone());
        let mut variant = proposal(body.clone());
        variant.causation = Causation::new(
            Source::Observer,
            Trigger::Observer { name: "other".into() },
        );
        assert_eq!(
            fingerprint_proposal(&Wf, &base).unwrap(),
            fingerprint_proposal(&Wf, &variant).unwrap(),
            "fingerprint drifted across causation variants",
        );
    }
}

#[tokio::test]
async fn fingerprint_matches_between_in_memory_and_file_repositories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let mem_repo = Arc::new(InMemoryRepository::<Wf>::new(Wf));

    let file_unit = UnitId::try_new("file").unwrap();
    let mem_unit = UnitId::try_new("mem").unwrap();

    let bodies = bodies();
    let file_report = file_repo
        .append(&file_unit, bodies.iter().cloned().map(proposal).collect(), AppendMode::BestEffort)
        .await
        .expect("file append");
    let mem_report = mem_repo
        .append(&mem_unit, bodies.iter().cloned().map(proposal).collect(), AppendMode::BestEffort)
        .await
        .expect("mem append");

    assert_eq!(file_report.accepted.len(), mem_report.accepted.len());

    let file_fps: Vec<Fingerprint> =
        file_report.accepted.iter().map(|e| fingerprint_event(&Wf, e).unwrap()).collect();
    let mem_fps: Vec<Fingerprint> =
        mem_report.accepted.iter().map(|e| fingerprint_event(&Wf, e).unwrap()).collect();
    assert_eq!(
        file_fps, mem_fps,
        "fingerprints diverged between FileRepository and InMemoryRepository",
    );
}
