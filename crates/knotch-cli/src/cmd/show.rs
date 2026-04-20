//! `knotch show <unit>` — read-only projection summary.
//!
//! Four formats via `--format`:
//!
//! - `summary` (default) — projection overview (phase, status, shipped count, event
//!   count).
//! - `brief` — one-line status (unit, phase, status). Machine- friendly default for
//!   scripts. Replaces the old `knotch status` subcommand.
//! - `raw` — full JSONL event stream, identical to `knotch log` output.
//! - `json` — structured JSON version of `summary`. Same as `--json` global flag applied
//!   to the default format.

use clap::{Args as ClapArgs, ValueEnum};
use knotch_kernel::{MilestoneKind as _, PhaseKind, Repository, UnitId, WorkflowKind};
use knotch_workflow::ConfigWorkflow;
use serde_json::json;

use crate::{cmd::OutputMode, config::Config};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, Default)]
pub(crate) enum Format {
    /// Multi-line projection overview (default).
    #[default]
    Summary,
    /// One-line status suitable for scripts.
    Brief,
    /// Raw JSONL event stream.
    Raw,
    /// Structured JSON of the summary.
    Json,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Unit slug to display.
    pub unit: String,
    /// Output representation.
    #[arg(long, value_enum, default_value_t = Format::Summary)]
    pub format: Format,
}

pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    // --json global flag collapses onto Json format.
    let format = if out.is_json() { Format::Json } else { args.format };
    let unit = UnitId::new(args.unit);
    let repo = config.build_repository()?;
    render::<ConfigWorkflow, _>(&repo, unit, format, &config.state_dir).await
}

async fn render<W, R>(
    repo: &R,
    unit: UnitId,
    format: Format,
    state_dir: &std::path::Path,
) -> anyhow::Result<()>
where
    W: WorkflowKind,
    R: Repository<W>,
{
    let log = repo.load(&unit).await?;
    let phase = knotch_kernel::project::current_phase(repo.workflow(), &log);
    let last_completed = knotch_kernel::project::last_completed_phase(&log);
    let status = knotch_kernel::project::current_status(&log);
    let shipped = knotch_kernel::project::shipped_milestones(&log);

    let render_phase = |p: Option<&W::Phase>| -> String {
        p.map(|p| PhaseKind::id(p).into_owned()).unwrap_or_else(|| "(none)".to_owned())
    };

    match format {
        Format::Summary => {
            println!("unit:               {}", unit.as_str());
            println!("current phase:      {}", render_phase(phase.as_ref()));
            println!("last completed:     {}", render_phase(last_completed.as_ref()));
            println!(
                "current status:     {}",
                status.as_ref().map(knotch_kernel::StatusId::as_str).unwrap_or("(none)")
            );
            println!("shipped milestones: {}", shipped.len());
            for m in &shipped {
                println!("  - {}", m.id());
            }
            println!("events recorded:    {}", log.events().len());
        }
        Format::Brief => {
            let phase_str = render_phase(phase.as_ref());
            let last_str = render_phase(last_completed.as_ref());
            let status_str =
                status.as_ref().map(knotch_kernel::StatusId::as_str).unwrap_or("(none)");
            println!(
                "{}\tphase={phase_str}\tlast={last_str}\tstatus={status_str}\tshipped={}\tevents={}",
                unit.as_str(),
                shipped.len(),
                log.events().len()
            );
        }
        Format::Raw => {
            let path = state_dir.join(unit.as_str()).join("log.jsonl");
            let lines = super::read_log_lines(&path).await?;
            for line in lines {
                println!("{line}");
            }
        }
        Format::Json => {
            let value = json!({
                "event": "show",
                "unit": unit.as_str(),
                "current_phase": phase.as_ref().map(|p| p.id().to_string()),
                "last_completed_phase": last_completed.as_ref().map(|p| p.id().to_string()),
                "current_status": status.as_ref().map(|s| s.as_str().to_string()),
                "shipped_milestones": shipped.iter().map(|m| m.id().to_string()).collect::<Vec<_>>(),
                "events_recorded": log.events().len(),
            });
            println!("{value}");
        }
    }
    Ok(())
}
