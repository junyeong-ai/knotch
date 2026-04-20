//! `knotch unit <init|use|list|current>` — workspace-local
//! active-unit management.

use anyhow::{Context as _, anyhow};
use clap::{Args as ClapArgs, Subcommand};
use knotch_agent::active::{ActiveUnit, resolve_active, write_active};
use knotch_kernel::UnitId;
use serde_json::json;

use crate::{cmd::OutputMode, config::Config};

#[derive(Debug, Subcommand)]
pub(crate) enum UnitCommand {
    /// Create a new unit directory and register its slug.
    Init(InitArgs),
    /// Set the active unit (writes `.knotch/active.toml`).
    Use(UseArgs),
    /// List every known unit under `state_dir`.
    List,
    /// Print the currently active unit slug.
    Current,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct InitArgs {
    /// Unit slug — kebab-case recommended.
    pub slug: String,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct UseArgs {
    /// Unit slug to make active.
    pub slug: String,
}

pub(crate) async fn run(config: &Config, out: OutputMode, cmd: UnitCommand) -> anyhow::Result<()> {
    match cmd {
        UnitCommand::Init(args) => run_init(config, out, args).await,
        UnitCommand::Use(args) => run_use(config, out, args).await,
        UnitCommand::List => run_list(config, out).await,
        UnitCommand::Current => run_current(config, out).await,
    }
}

async fn run_init(config: &Config, out: OutputMode, args: InitArgs) -> anyhow::Result<()> {
    let unit_dir = config.unit_dir(&args.slug);
    if unit_dir.exists() {
        return Err(anyhow!("unit `{}` already exists at {}", args.slug, unit_dir.display()));
    }
    tokio::fs::create_dir_all(&unit_dir)
        .await
        .with_context(|| format!("create unit dir {}", unit_dir.display()))?;
    match out {
        OutputMode::Human => println!("created unit `{}` at {}", args.slug, unit_dir.display()),
        OutputMode::Json => println!(
            "{}",
            json!({
                "event": "unit_init",
                "slug": args.slug,
                "dir": unit_dir.display().to_string(),
            })
        ),
    }
    Ok(())
}

async fn run_use(config: &Config, out: OutputMode, args: UseArgs) -> anyhow::Result<()> {
    let unit_dir = config.unit_dir(&args.slug);
    if !unit_dir.exists() {
        return Err(anyhow!(
            "unit `{}` has no directory at {} — run `knotch unit init {}` first",
            args.slug,
            unit_dir.display(),
            args.slug,
        ));
    }
    let unit = UnitId::new(args.slug.clone());
    write_active(&config.root, Some(&unit), "cli")
        .map_err(|e| anyhow!("write active.toml: {e}"))?;
    match out {
        OutputMode::Human => println!("active unit: `{}`", args.slug),
        OutputMode::Json => println!("{}", json!({"event": "unit_use", "slug": args.slug})),
    }
    Ok(())
}

async fn run_list(config: &Config, out: OutputMode) -> anyhow::Result<()> {
    let mut entries: Vec<String> = Vec::new();
    let mut rd = match tokio::fs::read_dir(&config.state_dir).await {
        Ok(r) => r,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return match out {
                OutputMode::Human => {
                    println!("(no units)");
                    Ok(())
                }
                OutputMode::Json => {
                    println!("{}", json!({"event": "unit_list", "units": Vec::<String>::new()}));
                    Ok(())
                }
            };
        }
        Err(e) => return Err(anyhow::Error::new(e).context("read state_dir")),
    };
    while let Some(entry) = rd.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_owned());
            }
        }
    }
    entries.sort();
    match out {
        OutputMode::Human => {
            if entries.is_empty() {
                println!("(no units)");
            } else {
                for e in &entries {
                    println!("{e}");
                }
            }
        }
        OutputMode::Json => println!("{}", json!({"event": "unit_list", "units": entries})),
    }
    Ok(())
}

async fn run_current(config: &Config, out: OutputMode) -> anyhow::Result<()> {
    let active = resolve_active(&config.root).map_err(|e| anyhow!("resolve active.toml: {e}"))?;
    match (active, out) {
        (ActiveUnit::Active(u), OutputMode::Human) => println!("{}", u.as_str()),
        (ActiveUnit::Active(u), OutputMode::Json) => {
            println!("{}", json!({"event": "unit_current", "slug": u.as_str()}));
        }
        (ActiveUnit::Uninitialized, OutputMode::Human) => println!("(none)"),
        (ActiveUnit::Uninitialized, OutputMode::Json) => {
            println!(
                "{}",
                json!({"event": "unit_current", "slug": null, "state": "uninitialized"})
            );
        }
        (ActiveUnit::NoProject, OutputMode::Human) => println!("(not in a knotch project)"),
        (ActiveUnit::NoProject, OutputMode::Json) => {
            println!("{}", json!({"event": "unit_current", "slug": null, "state": "no_project"}));
        }
    }
    Ok(())
}
