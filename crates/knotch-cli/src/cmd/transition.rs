//! `knotch transition <target>` — move the active unit to a new
//! lifecycle status.

use anyhow::{anyhow, bail};
use clap::Args as ClapArgs;
use knotch_kernel::{
    AppendMode, Causation, EventBody, Proposal, Rationale, Repository, StatusId, UnitId,
    WorkflowKind,
};
use knotch_workflow::ConfigWorkflow;
use serde::Serialize;

use crate::{cmd::{OutputMode, mark}, config::Config};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Target status (e.g. `shipped`, `archived`, `handed_off`).
    pub target: String,
    /// Bypass the "required phases resolved" invariant. Requires
    /// `--reason`.
    #[arg(long)]
    pub forced: bool,
    /// Rationale — mandatory when `--forced` is set.
    #[arg(long)]
    pub reason: Option<String>,
}

pub(crate) async fn run(
    config: &Config,
    out: OutputMode,
    args: Args,
) -> anyhow::Result<()> {
    let unit = mark::active_unit_or_bail(&config.root)?;
    let causation = Causation::cli("transition");
    let target = StatusId::new(args.target.clone());
    let repo = config.build_repository()?;
    warn_unknown_status(repo.workflow(), &args.target);
    append_transition::<ConfigWorkflow, _>(&repo, &unit, target, args, causation, out).await
}

/// Advisory warning when the target status is not in the workflow's
/// canonical vocabulary. The kernel still accepts the transition —
/// StatusId is open-universe by design — but a typo is the far
/// more common explanation.
fn warn_unknown_status<W: knotch_kernel::WorkflowKind>(workflow: &W, target: &str) {
    let known = workflow.known_statuses();
    if known.is_empty() || known.iter().any(|s| s.as_ref() == target) {
        return;
    }
    eprintln!(
        "warning: `{target}` is not a canonical status for workflow `{}`",
        workflow.name()
    );
    let joined = known
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("  canonical: {joined}");
    eprintln!("  proceeding — StatusId accepts any string. Use --forced with a rationale for custom statuses.");
}

async fn append_transition<W, R>(
    repo: &R,
    unit: &UnitId,
    target: StatusId,
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
    let rationale = match (args.forced, args.reason.as_ref()) {
        (true, None) => bail!("--forced transition requires --reason"),
        (_, Some(text)) => Some(
            Rationale::with_min(text.clone(), repo.workflow().min_rationale_chars())
                .map_err(|e| anyhow!(e))?,
        ),
        (false, None) => None,
    };
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::StatusTransitioned {
            target: target.clone(),
            forced: args.forced,
            rationale,
        },
        supersedes: None,
    };
    let report = repo
        .append(unit, vec![proposal], AppendMode::AllOrNothing)
        .await?;
    mark::emit_report(out, "status_transitioned", target.as_str(), &report);
    Ok(())
}
