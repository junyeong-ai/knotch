//! `knotch supersede <unit> <event> <rationale>` — append an
//! `EventSuperseded` entry that marks a prior event no-longer-
//! effective. Non-destructive: the target event stays on the log;
//! built-in projections call `effective_events` to skip superseded
//! entries.

use std::str::FromStr as _;

use anyhow::anyhow;
use clap::Args as ClapArgs;
use knotch_kernel::{
    AppendMode, Causation, EventBody, EventId, Proposal, Rationale, Repository, UnitId,
    WorkflowKind,
};
use knotch_workflow::ConfigWorkflow;
use serde::Serialize;

use crate::{
    cmd::{OutputMode, mark},
    config::Config,
};

/// `knotch supersede` arguments. All positional — mirrors the
/// `knotch gate <gate-id> <decision> <rationale>` pattern.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Unit owning the target event.
    pub unit: String,
    /// Event id to supersede (UUIDv7, as rendered by `knotch log`).
    pub event: String,
    /// Non-empty rationale — minimum length enforced by the
    /// active workflow's `min_rationale_chars`.
    pub rationale: String,
}

/// Run the supersede command.
///
/// # Errors
/// Returns on unknown event id, rationale shorter than the
/// workflow's minimum, missing target event, already-superseded
/// target, or any lower-level `Repository::append` failure.
pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let unit = UnitId::new(&args.unit);
    let target = EventId::from_str(&args.event)
        .map_err(|e| anyhow!("invalid event id `{}`: {e}", args.event))?;
    let causation = Causation::cli("supersede");
    let repo = config.build_repository()?;
    append_supersede::<ConfigWorkflow, _>(&repo, &unit, target, args, causation, out).await
}

async fn append_supersede<W, R>(
    repo: &R,
    unit: &UnitId,
    target: EventId,
    args: Args,
    causation: Causation,
    out: OutputMode,
) -> anyhow::Result<()>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let reason = Rationale::with_min(args.rationale, repo.workflow().min_rationale_chars())
        .map_err(|e| anyhow!(e))?;
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::EventSuperseded { target, reason },
        supersedes: None,
    };
    let report = repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    // Same success-vs-rejected shape as other write subcommands.
    mark::emit_report(out, "event_superseded", args.event.as_str(), &report);
    Ok(())
}
