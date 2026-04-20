//! End-to-end vibe preset test.

#![allow(missing_docs)]

use std::sync::Arc;

use knotch_kernel::{
    AppendMode, Proposal, Repository, Scope, UnitId,
    event::{ArtifactList, CommitKind, CommitRef, EventBody},
};
use workflow_vibe_case_study::{
    Session, SummaryBudget, TaskId, Vibe, VibePhase, build_repository, summary_for_llm,
};

fn tool_causation(session: &Session) -> knotch_kernel::Causation {
    session.tool("edit_file", "call-1")
}

#[tokio::test]
async fn agent_session_lifecycle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::try_new("signup-refactor").unwrap();

    let session = Session::new("alice", "claude-opus-4-7", "claude-code/1.0");

    let proposals: Vec<Proposal<Vibe>> = vec![
        Proposal {
            causation: tool_causation(&session),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        },
        Proposal {
            causation: tool_causation(&session),
            extension: (),
            body: EventBody::PhaseCompleted {
                phase: VibePhase::Intent,
                artifacts: ArtifactList::default(),
            },
            supersedes: None,
        },
        Proposal {
            causation: tool_causation(&session),
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
}

#[tokio::test]
async fn summary_includes_phase() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(build_repository(dir.path()));
    let unit = UnitId::try_new("signup-refactor").unwrap();
    let session = Session::new("alice", "claude-opus-4-7", "claude-code/1.0");

    repo.append(
        &unit,
        vec![Proposal {
            causation: tool_causation(&session),
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
    assert!(summary.approx_tokens > 0);
}
