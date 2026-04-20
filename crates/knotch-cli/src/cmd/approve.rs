//! `knotch approve <unit> <event> <decision> <rationale> --as <person>` —
//! append an `ApprovalRecorded` entry signed by a named human.
//!
//! Designed for human-in-the-loop workflows: an agent proposes a
//! gate decision / status transition / milestone; a reviewer or
//! operator ratifies (or refuses) it explicitly. The approval lands
//! as a first-class event so projections and queries see the
//! signature. Every approval carries a `Rationale` (bounded by the
//! workflow's `min_rationale_chars`) so the audit trail is usable.

use std::str::FromStr as _;

use anyhow::anyhow;
use clap::{Args as ClapArgs, ValueEnum};
use compact_str::CompactString;
use knotch_kernel::{
    AppendMode, Causation, Decision, EventBody, EventId, Proposal, Rationale, Repository, UnitId,
    WorkflowKind,
};
use knotch_workflow::ConfigWorkflow;
use serde::Serialize;

use crate::{
    cmd::{OutputMode, mark},
    config::Config,
};

/// CLI-level decision ValueEnum — mirrors the kernel `Decision`
/// vocabulary used by `knotch gate` so a reviewer's approval shape
/// matches what gates already record.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub(crate) enum DecisionArg {
    /// Endorse the target event — reviewer signs off.
    Approved,
    /// Refuse the target event — reviewer disagrees with it.
    Rejected,
    /// Defer — needs more context / follow-up before a final call.
    NeedsRevision,
    /// Parked — decision postponed to a later review cycle.
    Deferred,
}

impl From<DecisionArg> for Decision {
    fn from(d: DecisionArg) -> Self {
        match d {
            DecisionArg::Approved => Decision::Approved,
            DecisionArg::Rejected => Decision::Rejected,
            DecisionArg::NeedsRevision => Decision::NeedsRevision,
            DecisionArg::Deferred => Decision::Deferred,
        }
    }
}

/// `knotch approve` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Unit owning the target event.
    pub unit: String,
    /// Event id to approve (UUIDv7, as rendered by `knotch log`).
    pub event: String,
    /// Decision to record — `approved` / `rejected` /
    /// `needs-revision` / `deferred`.
    #[arg(value_enum)]
    pub decision: DecisionArg,
    /// Non-empty rationale explaining the decision. Minimum length
    /// enforced by the active workflow's `min_rationale_chars`.
    pub rationale: String,
    /// Named human signing the approval. Stored as a plain
    /// `CompactString` — duplicate detection uses exact equality.
    #[arg(long = "as")]
    pub approver: String,
}

/// Run the approve command.
///
/// # Errors
/// Returns on unknown event id, rationale shorter than the
/// workflow's minimum, missing target event, duplicate approval
/// from the same approver, or any lower-level `Repository::append`
/// failure.
pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let unit = UnitId::try_new(&args.unit)
        .map_err(|e| anyhow!("invalid unit slug `{}`: {e}", args.unit))?;
    let target = EventId::from_str(&args.event)
        .map_err(|e| anyhow!("invalid event id `{}`: {e}", args.event))?;
    let causation = Causation::cli("approve");
    let repo = config.build_repository()?;
    append_approval::<ConfigWorkflow, _>(&repo, &unit, target, args, causation, out).await
}

async fn append_approval<W, R>(
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
    if args.approver.trim().is_empty() {
        return Err(anyhow!("--as <person> must not be empty"));
    }
    let rationale = Rationale::with_min(args.rationale, repo.workflow().min_rationale_chars())
        .map_err(|e| anyhow!(e))?;
    let approver = CompactString::from(args.approver.as_str());
    let decision: Decision = args.decision.into();
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::ApprovalRecorded { target, approver, decision, rationale },
        supersedes: None,
    };
    let report = repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    mark::emit_report(out, "approval_recorded", args.event.as_str(), &report);
    Ok(())
}
