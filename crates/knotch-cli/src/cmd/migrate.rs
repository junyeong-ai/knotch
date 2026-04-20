//! `knotch migrate` — advertise schema version and validate JSONL.
//!
//! Phase 8 ships migration *detection* — identifying units whose
//! header schema differs from `knotch_proto::SCHEMA_VERSION`. Actual
//! migrator chains are user-supplied via `knotch-proto`; the CLI
//! gains a `--to` target that consults `knotch_proto::migration::Registry`
//! in Phase 9 when presets populate it.

use std::path::PathBuf;

use anyhow::Context as _;
use clap::Args as ClapArgs;
use serde_json::{Value, json};

use crate::{cmd::OutputMode, config::Config};

/// `knotch migrate` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Only inspect this unit's log. Omit to scan every unit.
    pub unit: Option<String>,
    /// Target schema version. Phase 8 only reports mismatches — run
    /// a preset-specific CLI for the actual rewrite.
    #[arg(long = "to")]
    pub target: Option<u32>,
}

/// Run the migrate command (detection-only in Phase 8).
///
/// # Errors
/// Returns an error if the state directory cannot be walked.
pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let target = args.target.unwrap_or(knotch_proto::SCHEMA_VERSION);
    let mut findings: Vec<Finding> = Vec::new();
    match args.unit.as_deref() {
        Some(unit) => {
            findings.push(inspect_unit(config, unit).await?);
        }
        None => {
            let units = enumerate_units(&config.state_dir).await?;
            for unit in units {
                findings.push(inspect_unit(config, &unit).await?);
            }
        }
    }

    let mismatches: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.schema_version.is_some() && f.schema_version != Some(target))
        .collect();

    match out {
        OutputMode::Human => {
            println!("target schema version: {target}");
            if findings.is_empty() {
                println!("(no units found)");
                return Ok(());
            }
            for f in &findings {
                let tag = match f.schema_version {
                    None => "missing-header".to_owned(),
                    Some(v) if v == target => "ok".to_owned(),
                    Some(v) => format!("mismatch (v{v})"),
                };
                println!("  {:<30} {tag}", f.unit);
            }
            if mismatches.is_empty() {
                println!("all units at target version");
            } else {
                println!("{} unit(s) require a preset-specific migration", mismatches.len());
            }
        }
        OutputMode::Json => {
            let entries: Vec<Value> = findings
                .iter()
                .map(|f| {
                    json!({
                        "unit": f.unit,
                        "schema_version": f.schema_version,
                        "target": target,
                        "mismatch": f.schema_version.is_some() && f.schema_version != Some(target),
                        "log": f.log_path.display().to_string(),
                    })
                })
                .collect();
            let value = json!({
                "target": target,
                "findings": entries,
                "mismatches": mismatches.len(),
            });
            println!("{value}");
        }
    }

    Ok(())
}

#[derive(Debug)]
struct Finding {
    unit: String,
    schema_version: Option<u32>,
    log_path: PathBuf,
}

async fn enumerate_units(state_dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let storage = knotch_storage::FileSystemStorage::new(state_dir);
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(state_dir).await else {
        return Ok(out);
    };
    while let Some(entry) = entries.next_entry().await.context("read state dir")? {
        let ty = entry.file_type().await.context("stat entry")?;
        if ty.is_dir() {
            let slug = entry.file_name().to_string_lossy().into_owned();
            let Ok(unit_id) = knotch_kernel::UnitId::try_new(slug.as_str()) else {
                continue;
            };
            let log_path = storage.log_path(&unit_id);
            if tokio::fs::metadata(&log_path).await.is_ok() {
                out.push(slug);
            }
        }
    }
    out.sort();
    Ok(out)
}

async fn inspect_unit(config: &Config, unit: &str) -> anyhow::Result<Finding> {
    let log_path = config.unit_log(unit);
    let body = tokio::fs::read_to_string(&log_path)
        .await
        .with_context(|| format!("read {}", log_path.display()))?;
    let header_version = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("kind").and_then(Value::as_str) == Some("__header__"))
        .and_then(|v| v.get("schema_version").and_then(Value::as_u64))
        .and_then(|n| u32::try_from(n).ok());
    Ok(Finding { unit: unit.to_owned(), schema_version: header_version, log_path })
}
