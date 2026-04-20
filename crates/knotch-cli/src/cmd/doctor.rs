//! `knotch doctor` — workspace health checks.

use std::path::Path;

use anyhow::Context as _;
use clap::Args as ClapArgs;
use serde_json::{Value, json};

use crate::{cmd::OutputMode, config::Config};

/// `knotch doctor` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {}

/// One check result.
#[derive(Debug, Clone)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Ok => "[ OK ]",
            Self::Warn => "[WARN]",
            Self::Fail => "[FAIL]",
        }
    }
}

/// Run the doctor command.
///
/// # Errors
/// Returns an error only on catastrophic failures (e.g. unable to
/// stat directories). Individual check failures surface in the
/// report, not as hard errors.
pub(crate) async fn run(config: &Config, out: OutputMode, _args: Args) -> anyhow::Result<()> {
    let mut checks = Vec::new();

    checks.push(check_directory("root", &config.root).await);
    checks.push(check_directory("state_dir", &config.state_dir).await);
    checks.push(check_config_file(&config.config_path()).await);
    checks.push(check_workflow(config));
    checks.push(check_observers(config));
    checks.push(check_units(config).await?);
    checks.push(check_unit_created_anchors(config).await?);
    checks.push(check_gitignore(&config.root).await);
    checks.push(check_queue_stale(&config.root).await);
    checks.push(check_secret_scanner(&config.root).await);
    checks.push(check_agent_env());

    let has_fail = checks.iter().any(|c| c.status == Status::Fail);

    match out {
        OutputMode::Human => {
            println!("knotch doctor:");
            for c in &checks {
                println!("  {} {:<14} {}", c.status.symbol(), c.name, c.detail);
            }
            if has_fail {
                println!("one or more checks failed — see messages above");
            }
        }
        OutputMode::Json => {
            let entries: Vec<Value> = checks
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "status": c.status.tag(),
                        "detail": c.detail,
                    })
                })
                .collect();
            let value = json!({ "checks": entries, "ok": !has_fail });
            println!("{value}");
        }
    }

    if has_fail {
        anyhow::bail!(
            "doctor found {} failure(s)",
            checks.iter().filter(|c| c.status == Status::Fail).count()
        );
    }
    Ok(())
}

async fn check_directory(name: &'static str, path: &Path) -> Check {
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_dir() => {
            Check { name, status: Status::Ok, detail: format!("{} (dir)", path.display()) }
        }
        Ok(_) => Check {
            name,
            status: Status::Fail,
            detail: format!("{} exists but is not a directory", path.display()),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Check {
            name,
            status: Status::Warn,
            detail: format!("{} does not exist (run `knotch init`)", path.display()),
        },
        Err(err) => {
            Check { name, status: Status::Fail, detail: format!("{}: {err}", path.display()) }
        }
    }
}

fn check_workflow(config: &Config) -> Check {
    match config.load_workflow() {
        Ok(w) => {
            use knotch_kernel::WorkflowKind;
            let standard = w.required_phases(&knotch_kernel::Scope::Standard);
            let phase_count = standard.len();
            let known = w.known_statuses().len();
            Check {
                name: "workflow",
                status: Status::Ok,
                detail: format!(
                    "{} (schema v{}, {phase_count} phases, {known} statuses)",
                    w.name(),
                    w.schema_version(),
                ),
            }
        }
        Err(err) => {
            Check { name: "workflow", status: Status::Fail, detail: format!("load failed: {err}") }
        }
    }
}

fn check_observers(config: &Config) -> Check {
    match config.load_observer_manifests() {
        Ok(ms) if ms.is_empty() => Check {
            name: "observers",
            status: Status::Ok,
            detail: "no [[observers]] declared".to_owned(),
        },
        Ok(ms) => {
            let names: Vec<String> = ms.iter().map(|m| m.name.to_string()).collect();
            let missing: Vec<&str> =
                ms.iter().filter(|m| !m.binary.exists()).map(|m| m.name.as_str()).collect();
            if missing.is_empty() {
                Check {
                    name: "observers",
                    status: Status::Ok,
                    detail: format!("{} declared: {}", ms.len(), names.join(", ")),
                }
            } else {
                Check {
                    name: "observers",
                    status: Status::Fail,
                    detail: format!(
                        "{} declared, {} with missing binary: {}",
                        ms.len(),
                        missing.len(),
                        missing.join(", "),
                    ),
                }
            }
        }
        Err(err) => Check {
            name: "observers",
            status: Status::Fail,
            detail: format!("parse failed: {err}"),
        },
    }
}

async fn check_config_file(path: &Path) -> Check {
    match tokio::fs::read_to_string(path).await {
        Ok(body) => match toml::from_str::<toml::Value>(&body) {
            Ok(_) => Check {
                name: "knotch.toml",
                status: Status::Ok,
                detail: format!("{} parses", path.display()),
            },
            Err(err) => Check {
                name: "knotch.toml",
                status: Status::Fail,
                detail: format!("{}: {err}", path.display()),
            },
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Check {
            name: "knotch.toml",
            status: Status::Warn,
            detail: format!("{} not found (run `knotch init`)", path.display()),
        },
        Err(err) => Check {
            name: "knotch.toml",
            status: Status::Fail,
            detail: format!("{}: {err}", path.display()),
        },
    }
}

async fn check_units(config: &Config) -> anyhow::Result<Check> {
    let state = &config.state_dir;
    let Ok(mut entries) = tokio::fs::read_dir(state).await else {
        return Ok(Check {
            name: "units",
            status: Status::Warn,
            detail: format!("no state dir at {}", state.display()),
        });
    };
    let mut healthy = 0usize;
    let mut broken = 0usize;
    let mut broken_units: Vec<String> = Vec::new();
    while let Some(entry) = entries.next_entry().await.context("read state dir")? {
        if entry.file_type().await.context("stat entry")?.is_dir() {
            let unit = entry.file_name().to_string_lossy().into_owned();
            let log_path = entry.path().join("log.jsonl");
            if tokio::fs::metadata(&log_path).await.is_err() {
                continue;
            }
            match probe_unit(&log_path).await {
                Ok(()) => healthy += 1,
                Err(_) => {
                    broken += 1;
                    broken_units.push(unit);
                }
            }
        }
    }
    let status = if broken > 0 { Status::Fail } else { Status::Ok };
    Ok(Check {
        name: "units",
        status,
        detail: if broken > 0 {
            format!("{healthy} healthy, {broken} broken: {}", broken_units.join(", "))
        } else {
            format!("{healthy} healthy")
        },
    })
}

async fn probe_unit(log_path: &Path) -> anyhow::Result<()> {
    let body = tokio::fs::read_to_string(log_path).await?;
    for (idx, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<serde_json::Value>(line)
            .with_context(|| format!("parse line {} of {}", idx + 1, log_path.display()))?;
    }
    Ok(())
}

/// Warn when a unit's log lacks a `UnitCreated` anchor. Without the
/// anchor the kernel's scope-driven preconditions (notably
/// `TerminalTransitionRequiresRequiredPhases`) silently skip because
/// they read scope from the anchor's body. `knotch unit init <slug>`
/// is the repair path — re-running it on a dir that exists but has
/// no anchor emits the anchor in place.
async fn check_unit_created_anchors(config: &Config) -> anyhow::Result<Check> {
    let state = &config.state_dir;
    let Ok(mut entries) = tokio::fs::read_dir(state).await else {
        return Ok(Check { name: "anchors", status: Status::Ok, detail: "no state dir".into() });
    };
    let mut missing: Vec<String> = Vec::new();
    while let Some(entry) = entries.next_entry().await.context("read state dir")? {
        if !entry.file_type().await.context("stat entry")?.is_dir() {
            continue;
        }
        let log_path = entry.path().join("log.jsonl");
        let Ok(body) = tokio::fs::read_to_string(&log_path).await else { continue };
        let has_anchor = body.lines().filter(|l| !l.trim().is_empty()).any(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("body").and_then(|b| b.get("type")).cloned())
                .is_some_and(|t| t.as_str() == Some("unit_created"))
        });
        if !has_anchor {
            missing.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    if missing.is_empty() {
        Ok(Check {
            name: "anchors",
            status: Status::Ok,
            detail: "every unit has a UnitCreated anchor".into(),
        })
    } else {
        missing.sort();
        Ok(Check {
            name: "anchors",
            status: Status::Warn,
            detail: format!(
                "{} unit(s) missing UnitCreated — scope-driven checks dormant; \
                 repair with `knotch unit init <slug>`: {}",
                missing.len(),
                missing.join(", "),
            ),
        })
    }
}

/// Warn when `.knotch/` is not in `.gitignore`. Committing runtime
/// state pollutes history with per-machine pointers and is easy to
/// do by accident.
async fn check_gitignore(root: &Path) -> Check {
    let path = root.join(".gitignore");
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let ignored = body
                .lines()
                .any(|l| matches!(l.trim(), ".knotch" | ".knotch/" | "/.knotch" | "/.knotch/"));
            if ignored {
                Check { name: ".gitignore", status: Status::Ok, detail: "contains .knotch/".into() }
            } else {
                Check {
                    name: ".gitignore",
                    status: Status::Warn,
                    detail: "missing `.knotch/` — runtime state may leak into commits".into(),
                }
            }
        }
        Err(_) => Check {
            name: ".gitignore",
            status: Status::Warn,
            detail: "not found — re-run `knotch init` to add `.knotch/`".into(),
        },
    }
}

/// Flag queue entries older than 24h. Usually benign (one stale
/// drain after a preset switch), but reliable enough to surface.
async fn check_queue_stale(root: &Path) -> Check {
    let queue = root.join(".knotch").join("queue");
    let Ok(mut entries) = tokio::fs::read_dir(&queue).await else {
        return Check { name: "queue", status: Status::Ok, detail: "empty".into() };
    };
    let threshold = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(24 * 60 * 60))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let mut total = 0usize;
    let mut stale = 0usize;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path().extension().is_none_or(|e| e != "json") {
            continue;
        }
        total += 1;
        if let Ok(meta) = entry.metadata().await {
            if let Ok(mtime) = meta.modified() {
                if mtime < threshold {
                    stale += 1;
                }
            }
        }
    }
    if total == 0 {
        return Check { name: "queue", status: Status::Ok, detail: "empty".into() };
    }
    if stale > 0 {
        return Check {
            name: "queue",
            status: Status::Warn,
            detail: format!(
                "{total} entry(ies), {stale} older than 24h — \
                 run `knotch reconcile` or investigate preset mismatch"
            ),
        };
    }
    Check {
        name: "queue",
        status: Status::Ok,
        detail: format!("{total} entry(ies), drains pending"),
    }
}

/// Warn when `KNOTCH_MODEL` / `KNOTCH_HARNESS` are unset —
/// `hook_causation` falls back to `"unknown"` / `"claude-code"`
/// respectively, which loses fidelity in downstream attribution
/// queries.
fn check_agent_env() -> Check {
    let model = std::env::var("KNOTCH_MODEL").ok();
    let harness = std::env::var("KNOTCH_HARNESS").ok();
    match (model.as_deref(), harness.as_deref()) {
        (Some(m), Some(h)) if !m.is_empty() && !h.is_empty() => Check {
            name: "agent env",
            status: Status::Ok,
            detail: format!("KNOTCH_MODEL={m} KNOTCH_HARNESS={h}"),
        },
        (Some(m), None) if !m.is_empty() => Check {
            name: "agent env",
            status: Status::Warn,
            detail: format!(
                "KNOTCH_MODEL={m} but KNOTCH_HARNESS unset — harness will record as `claude-code`"
            ),
        },
        _ => Check {
            name: "agent env",
            status: Status::Warn,
            detail: "KNOTCH_MODEL / KNOTCH_HARNESS unset — export them in your shell \
                     profile so hook causations record accurate attribution"
                .into(),
        },
    }
}

/// knotch is **not** a secret scanner — see README. This check
/// nudges users towards a proper upstream tool (gitleaks,
/// trufflehog, detect-secrets, git-secrets) installed in
/// `.git/hooks/pre-commit`.
async fn check_secret_scanner(root: &Path) -> Check {
    let hook = root.join(".git").join("hooks").join("pre-commit");
    let body = tokio::fs::read_to_string(&hook).await.unwrap_or_default();
    let detected = ["gitleaks", "trufflehog", "detect-secrets", "git-secrets"]
        .iter()
        .find(|tool| body.contains(*tool));
    match detected {
        Some(tool) => Check {
            name: "secret scan",
            status: Status::Ok,
            detail: format!("`{tool}` wired into pre-commit"),
        },
        None => Check {
            name: "secret scan",
            status: Status::Warn,
            detail: "no scanner in .git/hooks/pre-commit — knotch does not scan; \
                     install gitleaks / trufflehog upstream of the knotch hook"
                .into(),
        },
    }
}
