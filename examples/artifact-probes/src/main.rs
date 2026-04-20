#![allow(missing_docs)]

//! # P1-2 example: artifact-existence probes
//!
//! Shows how an adopter plugs a filesystem probe into
//! `AppendContext::fs` so `PhaseCompleted` refuses to land when one
//! of its declared artifact paths is missing on disk.
//!
//! The Repository doesn't inject probes on its own — the caller wires
//! them in at the `Proposal` construction site. Here we do it
//! manually by calling `EventBody::check_precondition` with an
//! `AppendContext` carrying a custom probe. In production, observers
//! or CLI wrappers would compose this into their own
//! `proposal → precondition → append` flow.
//!
//! Run with `cargo run -p knotch-example-artifact-probes`.

use std::path::Path;

use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, Scope, UnitId,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, EventBody},
    precondition::{AppendContext, ArtifactCheck},
};
use knotch_workflow::{Knotch, KnotchPhase};

/// A filesystem probe that only considers paths inside a specific
/// directory to be "present". Adopters wire in their own probe that
/// resolves paths against the project root.
struct PresentInDir<'a>(&'a Path);

impl<'a> ArtifactCheck for PresentInDir<'a> {
    fn exists(&self, path: &Path) -> bool {
        self.0.join(path).exists()
    }
}

fn causation() -> Causation {
    Causation::new(
        Source::Cli,
        Principal::System { service: "artifact-probe-example".into() },
        Trigger::Manual,
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let artifact_dir = tempfile::tempdir()?;

    // Seed a unit.
    let repo = knotch_workflow::build_repository(state.path());
    let unit = UnitId::new("artifact-probe-demo");
    repo.append(
        &unit,
        vec![Proposal {
            causation: causation(),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        }],
        AppendMode::BestEffort,
    )
    .await?;

    // Build a `PhaseCompleted` that declares `plan.md` as its artifact.
    let body: EventBody<Knotch> = EventBody::PhaseCompleted {
        phase: KnotchPhase::Plan,
        artifacts: ArtifactList(vec!["plan.md".into()]),
    };

    // First run: plan.md doesn't exist yet → probe refuses.
    let log = repo.load(&unit).await?;
    let probe = PresentInDir(artifact_dir.path());
    let ctx = AppendContext::<Knotch>::new(&Knotch, &log).with_fs(&probe);
    match body.check_precondition(&ctx) {
        Err(knotch_kernel::error::PreconditionError::ArtifactMissing { path }) => {
            println!("probe blocked append: artifact missing at `{path}`");
        }
        other => anyhow::bail!("expected ArtifactMissing, got {other:?}"),
    }

    // Create the artifact, retry: now the probe admits the proposal.
    std::fs::write(artifact_dir.path().join("plan.md"), "# plan")?;
    let log = repo.load(&unit).await?;
    let ctx = AppendContext::<Knotch>::new(&Knotch, &log).with_fs(&probe);
    body.check_precondition(&ctx).expect("probe admits the proposal once plan.md exists");
    println!("probe admitted PhaseCompleted after plan.md materialised");

    // The precondition having succeeded, the caller now appends the
    // event as usual.
    repo.append(
        &unit,
        vec![Proposal { causation: causation(), extension: (), body, supersedes: None }],
        AppendMode::BestEffort,
    )
    .await?;
    let log = repo.load(&unit).await?;
    println!("events recorded: {}", log.events().len());
    Ok(())
}
