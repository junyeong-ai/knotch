//! Phase 5 exit criterion: 10× reconcile loop on the same state
//! produces zero new events after the first pass.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use jiff::Timestamp;
use knotch_derive::MilestoneKind;
use knotch_kernel::{
    AppendMode, Causation, PhaseKind, Proposal, Repository, Scope, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{CommitKind, EventBody, SkipKind},
    project::effective_events,
};
use knotch_observer::GitLogObserver;
use knotch_reconciler::Reconciler;
use knotch_testing::{InMemoryRepository, InMemoryVcs, VcsFixture};
use serde::{Deserialize, Serialize};

// --- Minimal workflow for this test ----------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum TestPhase { Only }

impl PhaseKind for TestPhase {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed("only") }
    fn is_skippable(&self, _: &SkipKind) -> bool { false }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
pub enum TestMilestone { ShipSignup, FixPayments }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TestGate {}
impl knotch_kernel::GateKind for TestGate {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed("none") }
}

#[derive(Debug, Clone, Copy)]
pub struct TestWorkflow;

const PHASES: [TestPhase; 1] = [TestPhase::Only];
impl WorkflowKind for TestWorkflow {
    type Phase = TestPhase;
    type Milestone = TestMilestone;
    type Gate = TestGate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("test-workflow") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

// --- Fixture builder -------------------------------------------------

fn build_vcs() -> Arc<InMemoryVcs> {
    let vcs = InMemoryVcs::new();
    let t = Timestamp::from_second(1_700_000_000).expect("ts");
    vcs.push_commit(
        VcsFixture::verified(
            "aaaaaaaaaaaaaaaa".to_string() + "aaaaaaaaaaaaaaaaaaaaaaaa",
            "feat(signup): add OIDC",
            t,
        )
        .with_kind(CommitKind::Feat),
    );
    vcs.push_commit(
        VcsFixture::verified(
            "bbbbbbbbbbbbbbbb".to_string() + "bbbbbbbbbbbbbbbbbbbbbbbb",
            "fix(payments): correct decimal",
            t,
        )
        .with_kind(CommitKind::Fix),
    );
    vcs.set_head(knotch_kernel::event::CommitRef::new(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ));
    Arc::new(vcs)
}

fn unit_created_proposal() -> Proposal<TestWorkflow> {
    Proposal {
        causation: Causation::new(
            Source::Cli,
            Principal::System { service: "test".into() },
            Trigger::Manual,
        ),
        extension: (),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    }
}

fn resolver() -> knotch_observer::git_log::MilestoneResolver<TestWorkflow> {
    Arc::new(|parsed| match parsed.scope.as_deref() {
        Some("signup") => Some(TestMilestone::ShipSignup),
        Some("payments") => Some(TestMilestone::FixPayments),
        _ => None,
    })
}

// --- The test --------------------------------------------------------

#[tokio::test]
async fn ten_reconcile_passes_produce_zero_new_events_after_the_first() {
    let repo = Arc::new(InMemoryRepository::<TestWorkflow>::new(TestWorkflow));
    let unit = UnitId::new("test-unit");

    // Seed the log with a UnitCreated event so phase-invariants pass.
    repo.append(&unit, vec![unit_created_proposal()], AppendMode::BestEffort)
        .await
        .expect("seed");

    let vcs = build_vcs();
    let observer = Arc::new(GitLogObserver::<InMemoryVcs, TestWorkflow>::new(
        vcs.clone(),
        resolver(),
    ));
    let reconciler = Reconciler::<TestWorkflow, _>::builder(repo.clone())
        .observer(observer)
        .build();

    let first = reconciler.reconcile(&unit).await.expect("first");
    assert!(first.accepted_any(), "first pass must accept MilestoneShipped events");
    let first_accepted = first.append.accepted.len();
    assert_eq!(first_accepted, 2, "expected 2 feat/fix commits to produce 2 events");

    for pass in 2..=10 {
        let report = reconciler
            .reconcile(&unit)
            .await
            .unwrap_or_else(|e| panic!("pass {pass} failed: {e:?}"));
        assert!(
            !report.accepted_any(),
            "pass {pass} accepted new events — replay is not idempotent"
        );
        assert_eq!(
            report.rejected_count(),
            2,
            "pass {pass} should reject both proposals as duplicates"
        );
    }

    // Final log inspection — should contain UnitCreated + 2 MilestoneShipped.
    let log = repo.load(&unit).await.expect("load");
    let effective = effective_events(&log);
    assert_eq!(effective.len(), 3);
    let shipped: Vec<_> = effective
        .iter()
        .filter(|e| matches!(e.body, EventBody::MilestoneShipped { .. }))
        .collect();
    assert_eq!(shipped.len(), 2);
}
