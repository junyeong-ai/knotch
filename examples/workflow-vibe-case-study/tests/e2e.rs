//! End-to-end vibe preset test.

#![allow(missing_docs)]

use std::sync::Arc;

use knotch_kernel::{
    AppendMode, Proposal, Repository, Scope, UnitId,
    causation::{Cost, Trigger},
    event::{ArtifactList, CommitKind, CommitRef, EventBody},
};
use rust_decimal::Decimal;
use workflow_vibe_case_study::{
    Session, SummaryBudget, TaskId, Vibe, VibePhase, build_repository, summary_for_llm,
    total_tokens, total_usd,
};

fn tool_causation(session: &Session) -> knotch_kernel::Causation {
    session.tool("edit_file", "call-1")
}

fn cost_causation(
    session: &Session,
    usd: Decimal,
    tin: u32,
    tout: u32,
) -> knotch_kernel::Causation {
    session
        .causation(Trigger::ToolInvocation { tool: "bash".into(), call_id: "call-2".into() })
        .with_cost(Cost::new(Some(usd), tin, tout))
}

#[tokio::test]
async fn agent_session_lifecycle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::new("signup-refactor");

    let session = Session::new("alice", "claude-opus-4-7", "claude-code/1.0");

    let proposals: Vec<Proposal<Vibe>> = vec![
        Proposal {
            causation: tool_causation(&session),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        },
        Proposal {
            causation: cost_causation(&session, Decimal::new(15, 2), 1200, 500),
            extension: (),
            body: EventBody::PhaseCompleted {
                phase: VibePhase::Intent,
                artifacts: ArtifactList::default(),
            },
            supersedes: None,
        },
        Proposal {
            causation: cost_causation(&session, Decimal::new(42, 2), 3400, 1500),
            extension: (),
            body: EventBody::MilestoneShipped {
                milestone: TaskId("refactor-login".into()),
                commit: CommitRef::new("abcdef1234"),
                commit_kind: CommitKind::Refactor,
                status: knotch_kernel::CommitStatus::Verified,
            },
            supersedes: None,
        },
    ];

    repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("append");

    let log = repo.load(&unit).await.expect("load");
    assert_eq!(log.events().len(), 3);

    let (tin, tout) = total_tokens(&log);
    assert_eq!(tin, 1200 + 3400);
    assert_eq!(tout, 500 + 1500);
    assert_eq!(total_usd(&log), Some(Decimal::new(57, 2)));
}

#[tokio::test]
async fn summary_includes_phase_and_cost() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::new("signup-refactor");
    let session = Session::new("alice", "claude-opus-4-7", "claude-code/1.0");

    repo.append(
        &unit,
        vec![Proposal {
            causation: cost_causation(&session, Decimal::new(10, 2), 100, 50),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        }],
        AppendMode::BestEffort,
    )
    .await
    .expect("append");

    let log = repo.load(&unit).await.expect("load");
    let summary = summary_for_llm(&log, SummaryBudget::default());
    assert!(summary.body.contains("## knotch unit summary"));
    assert!(summary.body.contains("current phase"));
    assert!(summary.body.contains("cost so far"));
    assert!(summary.approx_tokens > 0);
}
