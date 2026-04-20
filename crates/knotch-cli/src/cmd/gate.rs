//! `knotch gate <gate-id> <decision> <rationale>` — record a gate
//! event with long-form rationale.

use std::str::FromStr as _;

use anyhow::anyhow;
use clap::Args as ClapArgs;
use knotch_kernel::{
    AppendMode, Causation, Decision, EventBody, Proposal, Rationale, Repository, UnitId,
    WorkflowKind,
};
use knotch_workflow::ConfigWorkflow;
use serde::Serialize;

use crate::{cmd::{OutputMode, mark}, config::Config};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Gate id (preset-specific — e.g. `g0-scope`, `intent_clear`).
    pub gate: String,
    /// Decision — `approved`, `rejected`, `needs_revision`,
    /// `deferred`.
    pub decision: String,
    /// Long-form rationale.
    pub rationale: String,
}

pub(crate) async fn run(
    config: &Config,
    out: OutputMode,
    args: Args,
) -> anyhow::Result<()> {
    let unit = mark::active_unit_or_bail(&config.root)?;
    let decision = Decision::from_str(&args.decision).map_err(|e| anyhow!(e))?;
    let causation = Causation::cli("gate");
    let repo = config.build_repository()?;
    append_gate::<ConfigWorkflow, _>(&repo, &unit, args, decision, causation, out).await
}

async fn append_gate<W, R>(
    repo: &R,
    unit: &UnitId,
    args: Args,
    decision: Decision,
    causation: Causation,
    out: OutputMode,
) -> anyhow::Result<()>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let gate = repo.workflow().parse_gate(&args.gate)
        .ok_or_else(|| anyhow!("unknown gate `{}` for preset `{}`", args.gate, repo.workflow().name()))?;
    let rationale = Rationale::with_min(args.rationale, repo.workflow().min_rationale_chars())
        .map_err(|e| anyhow!(e))?;
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::GateRecorded {
            gate,
            decision,
            rationale,
        },
        supersedes: None,
    };
    let report = repo
        .append(unit, vec![proposal], AppendMode::AllOrNothing)
        .await?;
    mark::emit_report(out, "gate_recorded", &args.gate, &report);
    Ok(())
}
