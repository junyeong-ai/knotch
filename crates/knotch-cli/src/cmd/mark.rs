//! `knotch mark <completed|skipped> <phase>` — skill-driven phase
//! events.
//!
//! Emits `PhaseCompleted` / `PhaseSkipped` against the active unit.
//! Preset dispatch picks the `WorkflowKind` impl; `parse_phase`
//! maps the caller-supplied name to the variant.

use std::{path::Path, str::FromStr as _};

use anyhow::{anyhow, bail};
use clap::{Args as ClapArgs, Subcommand};
use compact_str::CompactString;
use knotch_agent::active::{ActiveUnit, resolve_active};
use knotch_kernel::{
    AppendMode, AppendReport, Causation, EventBody, Proposal, Repository, UnitId, WorkflowKind,
    event::{ArtifactList, SkipKind},
};
use knotch_workflow::ConfigWorkflow;
use serde::Serialize;
use serde_json::json;

use crate::{cmd::OutputMode, config::Config};

#[derive(Debug, Subcommand)]
pub(crate) enum MarkCommand {
    /// Record `PhaseCompleted`.
    Completed(CompletedArgs),
    /// Record `PhaseSkipped`.
    Skipped(SkippedArgs),
}

#[derive(Debug, ClapArgs)]
pub(crate) struct CompletedArgs {
    /// Phase name (preset-specific — e.g. `implementation`, `verify`).
    pub phase: String,
    /// Artifact path to attach. Repeatable.
    #[arg(long)]
    pub artifact: Vec<String>,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct SkippedArgs {
    /// Phase name (preset-specific).
    pub phase: String,
    /// Skip reason — one of `scope_too_narrow`, `amnesty:<code>`, or
    /// any other string (mapped to `SkipKind::Custom { code }`).
    #[arg(long, required = true)]
    pub reason: String,
}

pub(crate) async fn run(config: &Config, out: OutputMode, cmd: MarkCommand) -> anyhow::Result<()> {
    let unit = active_unit_or_bail(&config.root)?;
    let causation = Causation::cli("mark");
    let repo = config.build_repository()?;
    match cmd {
        MarkCommand::Completed(a) => {
            append_completed::<ConfigWorkflow, _>(&repo, &unit, a, causation, out).await
        }
        MarkCommand::Skipped(a) => {
            append_skipped::<ConfigWorkflow, _>(&repo, &unit, a, causation, out).await
        }
    }
}

async fn append_completed<W, R>(
    repo: &R,
    unit: &UnitId,
    args: CompletedArgs,
    causation: Causation,
    out: OutputMode,
) -> anyhow::Result<()>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let phase = repo.workflow().parse_phase(&args.phase).ok_or_else(|| {
        anyhow!("unknown phase `{}` for preset `{}`", args.phase, repo.workflow().name())
    })?;
    let artifacts = ArtifactList(args.artifact.into_iter().map(CompactString::from).collect());
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::PhaseCompleted { phase, artifacts },
        supersedes: None,
    };
    let report = repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    emit_report(out, "phase_completed", &args.phase, &report);
    Ok(())
}

async fn append_skipped<W, R>(
    repo: &R,
    unit: &UnitId,
    args: SkippedArgs,
    causation: Causation,
    out: OutputMode,
) -> anyhow::Result<()>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let phase = repo.workflow().parse_phase(&args.phase).ok_or_else(|| {
        anyhow!("unknown phase `{}` for preset `{}`", args.phase, repo.workflow().name())
    })?;
    let reason = SkipKind::from_str(&args.reason).expect("SkipKind::from_str is infallible");
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::PhaseSkipped { phase, reason },
        supersedes: None,
    };
    let report = repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    emit_report(out, "phase_skipped", &args.phase, &report);
    Ok(())
}

pub(crate) fn active_unit_or_bail(root: &Path) -> anyhow::Result<UnitId> {
    match resolve_active(root).map_err(|e| anyhow!("resolve active: {e}"))? {
        ActiveUnit::Active(u) => Ok(u),
        ActiveUnit::Uninitialized => {
            bail!("no active unit — run `knotch unit use <id>` first")
        }
        ActiveUnit::NoProject => bail!("not in a knotch project (knotch.toml missing)"),
    }
}

pub(crate) fn emit_report<W>(out: OutputMode, event: &str, subject: &str, report: &AppendReport<W>)
where
    W: WorkflowKind,
{
    match out {
        OutputMode::Human => {
            println!(
                "{event}: {subject} — accepted={}, rejected={}",
                report.accepted.len(),
                report.rejected.len()
            );
            for rej in &report.rejected {
                println!("  rejected: {}", rej.reason);
            }
        }
        OutputMode::Json => {
            println!(
                "{}",
                json!({
                    "event": event,
                    "subject": subject,
                    "accepted": report.accepted.len(),
                    "rejected": report.rejected.iter().map(|r| r.reason.as_str()).collect::<Vec<_>>(),
                })
            );
        }
    }
}
