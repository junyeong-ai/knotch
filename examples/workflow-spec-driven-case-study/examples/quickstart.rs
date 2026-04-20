//! Quickstart example — ship a milestone through the SpecDriven
//! case-study workflow end-to-end.
//!
//! Run from the repo root:
//!
//! ```bash
//! cargo run --example quickstart -p workflow-spec-driven-case-study
//! ```

#![allow(missing_docs)]

use std::sync::Arc;

use compact_str::CompactString;
use knotch_kernel::{
    AppendMode, Causation, Repository, Scope, StatusId, UnitId,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, CommitKind, CommitRef},
    project::{current_phase, current_status, shipped_milestones},
};
use workflow_spec_driven_case_study::{SpecDriven, SpecPhase, StoryId, build_repository, events};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join("knotch-quickstart-demo");
    std::fs::create_dir_all(&root)?;
    let repo = Arc::new(build_repository(&root));
    let unit = UnitId::new("quickstart-1");

    let causation = || {
        Causation::new(
            Source::Cli,
            Principal::System { service: "quickstart".into() },
            Trigger::Manual,
        )
    };

    let proposals = vec![
        events::unit_created(causation(), Scope::Standard),
        events::phase_completed(causation(), SpecPhase::Specify, ArtifactList::default()),
        events::phase_completed(causation(), SpecPhase::Design, ArtifactList::default()),
        events::milestone_shipped(
            causation(),
            StoryId(CompactString::from("us1-signup")),
            CommitRef::new("abcd1234"),
            CommitKind::Feat,
        ),
        events::phase_completed(causation(), SpecPhase::Implement, ArtifactList::default()),
        events::phase_completed(causation(), SpecPhase::Review, ArtifactList::default()),
        events::phase_completed(causation(), SpecPhase::Wrapup, ArtifactList::default()),
        events::status_transitioned(causation(), StatusId::new("archived"), false, None),
    ];

    let report = repo.append(&unit, proposals, AppendMode::BestEffort).await?;
    println!("accepted: {}", report.accepted.len());
    println!("rejected: {}", report.rejected.len());

    let log = repo.load(&unit).await?;
    println!("events stored: {}", log.events().len());
    println!("current phase: {:?}", current_phase(&SpecDriven, &log));
    println!("current status: {:?}", current_status(&log));
    let shipped: Vec<String> =
        shipped_milestones::<SpecDriven>(&log).into_iter().map(|s| s.0.to_string()).collect();
    println!("shipped milestones: {shipped:?}");
    println!("state dir: {}", root.display());

    Ok(())
}
