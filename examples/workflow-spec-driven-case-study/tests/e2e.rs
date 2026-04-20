//! End-to-end spec-driven preset test against FileRepository.

#![allow(missing_docs)]

use std::sync::Arc;

use compact_str::CompactString;
use knotch_kernel::{
    AppendMode, Causation, Decision, Rationale, Repository, Scope, StatusId, UnitId,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, CommitKind, CommitRef, EventBody},
    project::{current_phase, current_status, shipped_milestones},
};
use workflow_spec_driven_case_study::{
    SpecDriven, SpecGate, SpecPhase, StoryId, build_repository, events,
};

fn causation() -> Causation {
    Causation::new(
        Source::Cli,
        Principal::System { service: "e2e".into() },
        Trigger::Command { name: "test".into() },
    )
}

#[tokio::test]
async fn full_lifecycle_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::try_new("story-001").unwrap();

    // SPECIFY → G0..G3 → DESIGN → IMPLEMENT → G5Review → REVIEW → WRAPUP.
    // The case-study workflow's gate ladder is kernel-enforced via
    // `SpecGate::prerequisites`: G5Review requires G0..G3.
    let gate = |g, r: &str| {
        events::gate_recorded(causation(), g, Decision::Approved, Rationale::new(r).unwrap())
    };
    let proposals = vec![
        events::unit_created(causation(), Scope::Standard),
        events::phase_completed(causation(), SpecPhase::Specify, ArtifactList::default()),
        gate(SpecGate::G0Scope, "scope assessed as standard"),
        gate(SpecGate::G1Clarify, "no NEEDS CLARIFICATION markers remain"),
        gate(SpecGate::G2Constitution, "plan passes constitution"),
        gate(SpecGate::G3Analyze, "analyze surfaced no criticals"),
        events::phase_completed(causation(), SpecPhase::Design, ArtifactList::default()),
        events::milestone_shipped(
            causation(),
            StoryId(CompactString::from("us1")),
            CommitRef::new("aaaabbbb"),
            CommitKind::Feat,
        ),
        events::phase_completed(causation(), SpecPhase::Implement, ArtifactList::default()),
        gate(SpecGate::G5Review, "reviewer approved on channel #review"),
        events::phase_completed(causation(), SpecPhase::Review, ArtifactList::default()),
        events::phase_completed(causation(), SpecPhase::Wrapup, ArtifactList::default()),
        events::status_transitioned(causation(), StatusId::new("archived"), false, None),
    ];

    let report = repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("append");
    assert_eq!(report.accepted.len(), 13);
    assert!(report.rejected.is_empty());

    let log = repo.load(&unit).await.expect("load");
    assert_eq!(log.events().len(), 13);
    assert_eq!(current_phase(&SpecDriven, &log), None); // all required phases resolved
    assert_eq!(current_status(&log).as_ref().map(StatusId::as_str), Some("archived"),);
    let shipped: Vec<String> =
        shipped_milestones::<SpecDriven>(&log).into_iter().map(|s| s.0.to_string()).collect();
    assert_eq!(shipped, vec!["us1".to_string()]);
}

#[tokio::test]
async fn tiny_scope_allows_skipping_review() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::try_new("tiny-123").unwrap();

    let phases: Vec<_> = SpecDriven.required_phases(&Scope::Tiny).to_vec();
    assert!(!phases.contains(&SpecPhase::Review));

    let mut proposals = vec![events::unit_created(causation(), Scope::Tiny)];
    for phase in phases {
        proposals.push(events::phase_completed(causation(), phase, ArtifactList::default()));
    }
    let report = repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("append");
    assert!(report.rejected.is_empty());

    let log = repo.load(&unit).await.expect("load");
    assert_eq!(current_phase(&SpecDriven, &log), None);
}

#[tokio::test]
async fn replay_persists_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = UnitId::try_new("persisted").unwrap();
    {
        let repo = build_repository(dir.path());
        repo.append(
            &unit,
            vec![events::unit_created(causation(), Scope::Standard)],
            AppendMode::BestEffort,
        )
        .await
        .expect("seed");
    }
    let repo = build_repository(dir.path());
    let log = repo.load(&unit).await.expect("load");
    assert_eq!(log.events().len(), 1);
    match &log.events()[0].body {
        EventBody::UnitCreated { scope } => assert_eq!(scope, &Scope::Standard),
        other => panic!("unexpected event: {other:?}"),
    }
}

// `SpecDriven` must implement `WorkflowKind` — make the compiler prove it
// in the test binary.
use knotch_kernel::WorkflowKind as _;
