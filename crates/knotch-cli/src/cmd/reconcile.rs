//! `knotch reconcile` — drain the `.knotch/queue/` backlog, run
//! declared subprocess observers against the active unit, and
//! optionally prune stale queue entries.
//!
//! The queue holds proposals whose `PostToolUse` append failed
//! (transient I/O, lock contention, Repository error). `SessionStart`
//! auto-drains on the next Claude Code session; this command does
//! the same drain on-demand.
//!
//! Observers declared in `knotch.toml`'s `[[observers]]` array run
//! against the active unit after the drain; `--queue-only` skips
//! them for operators who just want to drain and move on.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, anyhow};
use clap::Args as ClapArgs;
use knotch_agent::active::{ActiveUnit, resolve_active};
use knotch_kernel::{AppendMode, UnitId};
use knotch_observer::{DynObserver, SubprocessObserver};
use knotch_reconciler::Reconciler;
use knotch_workflow::ConfigWorkflow;
use serde_json::json;

use crate::{cmd::OutputMode, config::Config};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Delete every queue entry that remains after the drain. Use
    /// for a clean slate when `Proposal<ConfigWorkflow>` leftovers
    /// keep failing to deserialize.
    #[arg(long)]
    pub prune: bool,
    /// Delete queue entries older than `<hours>` hours. Combine
    /// with the drain to TTL out long-standing failures.
    #[arg(long, value_name = "HOURS")]
    pub prune_older: Option<u32>,
    /// Skip the observer pass — queue drain + prune only. Useful
    /// when observers haven't been declared yet or their binaries
    /// aren't available in this environment.
    #[arg(long)]
    pub queue_only: bool,
    /// Target a specific unit instead of the active one.
    #[arg(long, value_name = "UNIT")]
    pub unit: Option<String>,
}

pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let queue_dir = config.root.join(".knotch").join("queue");

    // 1. Drain — queued `Proposal<ConfigWorkflow>` replays.
    let repo = Arc::new(config.build_repository()?);
    let drained = knotch_agent::queue::drain::<ConfigWorkflow, _>(&queue_dir, &*repo).await?;

    // 2. Observer pass — subprocess observers declared in
    // `knotch.toml`, run against the resolved target unit.
    let observer_report = if args.queue_only {
        ObserverReport::Skipped
    } else {
        run_observers(config, repo.clone(), args.unit.as_deref()).await?
    };

    // 3. Optional prune — removes JSON files the drain left behind.
    let (pruned, pruned_mode) = if args.prune || args.prune_older.is_some() {
        let age = args.prune_older.map(|h| Duration::from_secs(u64::from(h) * 3600));
        let count = prune_queue(&queue_dir, age)?;
        let mode = match age {
            None => "all-remaining",
            Some(_) => "older-than",
        };
        (count, mode)
    } else {
        (0, "none")
    };

    match out {
        OutputMode::Human => {
            println!("reconcile:");
            println!("  drained:  {drained}");
            match &observer_report {
                ObserverReport::Skipped => {
                    println!("  observers: skipped (--queue-only)");
                }
                ObserverReport::NoUnit => {
                    println!("  observers: no active unit — skipped");
                }
                ObserverReport::NoObservers => {
                    println!("  observers: no [[observers]] declared");
                }
                ObserverReport::Ran {
                    unit,
                    observer_count,
                    accepted,
                    rejected,
                    observer_errors,
                } => {
                    println!("  observers: {observer_count} ran against `{unit}`");
                    println!("    proposals accepted: {accepted}");
                    if *rejected > 0 {
                        println!("    proposals rejected: {rejected}");
                    }
                    if !observer_errors.is_empty() {
                        println!("    observer errors: {}", observer_errors.len());
                        for err in observer_errors {
                            println!("      - {err}");
                        }
                    }
                }
            }
            if args.prune || args.prune_older.is_some() {
                if let Some(hours) = args.prune_older {
                    println!("  pruned:   {pruned} (older than {hours}h)");
                } else {
                    println!("  pruned:   {pruned} (all remaining)");
                }
            }
        }
        OutputMode::Json => {
            println!(
                "{}",
                json!({
                    "event": "reconcile",
                    "drained": drained,
                    "observers": observer_report.as_json(),
                    "pruned": pruned,
                    "prune_mode": pruned_mode,
                })
            );
        }
    }
    Ok(())
}

/// Observer-pass outcome for reporting purposes.
enum ObserverReport {
    Skipped,
    NoUnit,
    NoObservers,
    Ran {
        unit: String,
        observer_count: usize,
        accepted: usize,
        rejected: usize,
        observer_errors: Vec<String>,
    },
}

impl ObserverReport {
    fn as_json(&self) -> serde_json::Value {
        match self {
            Self::Skipped => json!({ "status": "skipped" }),
            Self::NoUnit => json!({ "status": "no_unit" }),
            Self::NoObservers => json!({ "status": "no_observers" }),
            Self::Ran { unit, observer_count, accepted, rejected, observer_errors } => json!({
                "status": "ran",
                "unit": unit,
                "observer_count": observer_count,
                "accepted": accepted,
                "rejected": rejected,
                "observer_errors": observer_errors,
            }),
        }
    }
}

async fn run_observers(
    config: &Config,
    repo: Arc<knotch_storage::FileRepository<ConfigWorkflow>>,
    explicit_unit: Option<&str>,
) -> anyhow::Result<ObserverReport> {
    let manifests = config.load_observer_manifests()?;
    if manifests.is_empty() {
        return Ok(ObserverReport::NoObservers);
    }

    let unit = if let Some(explicit) = explicit_unit {
        UnitId::try_new(explicit).map_err(|e| anyhow!("invalid unit slug `{explicit}`: {e}"))?
    } else {
        match resolve_active(&config.root).map_err(|e| anyhow!("resolve active: {e}"))? {
            ActiveUnit::Active(u) => u,
            ActiveUnit::Uninitialized | ActiveUnit::NoProject => {
                return Ok(ObserverReport::NoUnit);
            }
        }
    };

    let mut builder = Reconciler::builder(repo).append_mode(AppendMode::BestEffort);
    let observer_count = manifests.len();
    for manifest in manifests {
        let name = manifest.name.clone();
        let observer = SubprocessObserver::<ConfigWorkflow>::new(manifest)
            .with_context(|| format!("construct subprocess observer `{name}`"))?;
        builder = builder.observer(Arc::new(observer) as Arc<dyn DynObserver<ConfigWorkflow>>);
    }
    let reconciler = builder.build();
    let report = reconciler
        .reconcile(&unit)
        .await
        .with_context(|| format!("reconcile unit `{}`", unit.as_str()))?;

    let observer_errors: Vec<String> =
        report.observer_errors.iter().map(|f| format!("{}: {}", f.observer, f.source)).collect();

    Ok(ObserverReport::Ran {
        unit: unit.as_str().to_owned(),
        observer_count,
        accepted: report.append.accepted.len(),
        rejected: report.append.rejected.len(),
        observer_errors,
    })
}

/// Remove JSON queue entries.
///
/// - `older_than = Some(dur)` → only entries whose mtime is older than `now - dur`.
/// - `older_than = None` → every JSON entry (caller opted into `--prune` with no age
///   filter).
fn prune_queue(queue_dir: &Path, older_than: Option<Duration>) -> anyhow::Result<usize> {
    if !queue_dir.exists() {
        return Ok(0);
    }
    let cutoff =
        older_than.map(|dur| SystemTime::now().checked_sub(dur).unwrap_or(SystemTime::UNIX_EPOCH));
    let mut pruned = 0usize;
    for entry in std::fs::read_dir(queue_dir).context("read queue dir")? {
        let entry = entry.context("queue entry")?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let should_prune = match cutoff {
            None => true,
            Some(cutoff) => entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|mtime| mtime < cutoff),
        };
        if should_prune {
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            pruned += 1;
        }
    }
    Ok(pruned)
}
