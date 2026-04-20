// Examples are tutorial code, not a published library surface.
#![allow(missing_docs)]

//! Open-source pull-request workflow.
//!
//! - Phases: `Draft` → `Review` → `Merged`
//! - Milestone: `PrId` (PR number or slug)
//! - Gates: `CodeReview`, `MaintainerApproval`
//! - Terminal status: `merged` / `closed` / `abandoned`
//!
//! Walks a full PR lifecycle from draft submission to merge.

use compact_str::CompactString;
use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{
    AppendMode, Causation, Proposal, Rationale, Repository, Scope, StatusId, UnitId, WorkflowKind,
    event::{ArtifactList, EventBody},
    status::Decision,
};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum PrPhase {
    /// Author is still iterating.
    Draft,
    /// In review — reviewers request changes or approve.
    Review,
    /// Merged into the target branch.
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct PrId(pub CompactString);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum PrGate {
    /// Reviewer sign-off on the code.
    CodeReview,
    /// Maintainer green-light for merge.
    MaintainerApproval,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PrWorkflow;

const PHASES: [PrPhase; 3] = [PrPhase::Draft, PrPhase::Review, PrPhase::Merged];

impl WorkflowKind for PrWorkflow {
    type Phase = PrPhase;
    type Milestone = PrId;
    type Gate = PrGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("pr-workflow")
    }
    fn schema_version(&self) -> u32 {
        1
    }

    fn required_phases(&self, _scope: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }

    fn is_terminal_status(&self, status: &StatusId) -> bool {
        matches!(status.as_str(), "merged" | "closed" | "abandoned")
    }

    fn min_rationale_chars(&self) -> usize {
        // Gate rationales on PRs should be substantive.
        16
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = FileRepository::<PrWorkflow>::new(dir.path(), PrWorkflow);
    let unit = UnitId::try_new("pr-42").unwrap();

    append(&repo, &unit, EventBody::UnitCreated { scope: Scope::Standard }).await?;
    append(
        &repo,
        &unit,
        EventBody::PhaseCompleted { phase: PrPhase::Draft, artifacts: ArtifactList::default() },
    )
    .await?;
    append(
        &repo,
        &unit,
        EventBody::GateRecorded {
            gate: PrGate::CodeReview,
            decision: Decision::Approved,
            rationale: Rationale::new("LGTM — tests green and docs updated").unwrap(),
        },
    )
    .await?;
    append(
        &repo,
        &unit,
        EventBody::GateRecorded {
            gate: PrGate::MaintainerApproval,
            decision: Decision::Approved,
            rationale: Rationale::new("approved by maintainer for the 1.4 milestone").unwrap(),
        },
    )
    .await?;
    append(
        &repo,
        &unit,
        EventBody::PhaseCompleted { phase: PrPhase::Review, artifacts: ArtifactList::default() },
    )
    .await?;
    append(
        &repo,
        &unit,
        EventBody::PhaseCompleted { phase: PrPhase::Merged, artifacts: ArtifactList::default() },
    )
    .await?;
    append(
        &repo,
        &unit,
        EventBody::StatusTransitioned {
            target: StatusId::new("merged"),
            forced: false,
            rationale: None,
        },
    )
    .await?;

    let log = repo.load(&unit).await?;
    println!("pr:     {}", unit.as_str());
    println!("status: {:?}", knotch_kernel::project::current_status(&log));
    println!("events: {}", log.events().len());
    Ok(())
}

async fn append<R>(repo: &R, unit: &UnitId, body: EventBody<PrWorkflow>) -> anyhow::Result<()>
where
    R: Repository<PrWorkflow>,
{
    let proposal = Proposal {
        causation: Causation::cli("example-pr-workflow"),
        extension: (),
        body,
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    Ok(())
}
