// Examples are tutorial code, not a published library surface.
#![allow(missing_docs)]

//! Minimal custom `WorkflowKind`.
//!
//! Demonstrates the bare bones of a knotch workflow:
//!
//! - phases (`Start` → `Done`)
//! - free-form [`TaskId`] milestone
//! - single [`MiniGate::Review`] checkpoint
//! - empty extension (`()`)
//!
//! Run with `cargo run -p knotch-example-minimal`.

use compact_str::CompactString;
use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, Scope, UnitId, WorkflowKind,
    event::{ArtifactList, EventBody},
};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

/// Lifecycle phases.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum MiniPhase {
    /// Work begins.
    Start,
    /// Work complete.
    Done,
}

/// Task identifier — any string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct TaskId(pub CompactString);

/// Single review gate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum MiniGate {
    /// Peer / maintainer review.
    Review,
}

/// Workflow marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Minimal;

const PHASES: [MiniPhase; 2] = [MiniPhase::Start, MiniPhase::Done];

impl WorkflowKind for Minimal {
    type Phase = MiniPhase;
    type Milestone = TaskId;
    type Gate = MiniGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("minimal") }
    fn schema_version(&self) -> u32 { 1 }

    fn required_phases(&self, _scope: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }

    fn is_terminal_status(&self, status: &knotch_kernel::StatusId) -> bool {
        matches!(status.as_str(), "done" | "abandoned")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = FileRepository::<Minimal>::new(dir.path(), Minimal);
    let unit = UnitId::new("demo");

    append(&repo, &unit, EventBody::UnitCreated { scope: Scope::Standard }).await?;
    append(
        &repo,
        &unit,
        EventBody::PhaseCompleted {
            phase: MiniPhase::Start,
            artifacts: ArtifactList::default(),
        },
    )
    .await?;

    let log = repo.load(&unit).await?;
    println!("unit:          {}", unit.as_str());
    println!("current phase: {:?}", knotch_kernel::project::current_phase(&Minimal, &log));
    println!("events:        {}", log.events().len());
    Ok(())
}

async fn append<R>(repo: &R, unit: &UnitId, body: EventBody<Minimal>) -> anyhow::Result<()>
where
    R: Repository<Minimal>,
{
    let proposal = Proposal {
        causation: Causation::cli("example-minimal"),
        extension: (),
        body,
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::AllOrNothing)
        .await?;
    Ok(())
}
