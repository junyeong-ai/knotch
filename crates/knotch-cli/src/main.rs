//! `knotch` command-line tool.
//!
//! `hook` is the Claude Code integration surface; every other
//! subcommand is human- or skill-facing. The authoritative surface
//! is the `Command` enum below — run `knotch --help` to enumerate
//! it at runtime.

// `knotch-cli` is a binary crate; internal modules keep `pub(crate)`
// visibility. `missing_docs` is allowed because subcommand arg types
// use clap's derive macro which doesn't emit doc comments.
#![allow(missing_docs)]

mod cmd;
mod config;
mod home;

use std::{io, process::ExitCode};

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use tracing_subscriber::EnvFilter;

use crate::config::Config;

#[derive(Debug, Parser)]
#[command(
    name = "knotch",
    version,
    about = "Git-correlated event-sourced workflow state",
    long_about = None,
)]
struct Cli {
    /// Path to a knotch workspace root. Defaults to the first
    /// ancestor directory containing `knotch.toml`, falling back to
    /// the current working directory.
    #[arg(long, global = true, env = "KNOTCH_ROOT")]
    root: Option<std::path::PathBuf>,

    /// Emit machine-readable JSON rather than human-formatted output.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress tracing-subscriber output unless `RUST_LOG` is set.
    #[arg(long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a knotch workspace in the current directory.
    Init(cmd::init::Args),
    /// Print the event log for a unit.
    Log(cmd::log::Args),
    /// Run the reconciler against a unit.
    Reconcile(cmd::reconcile::Args),
    /// Diagnose workspace health.
    Doctor(cmd::doctor::Args),
    /// Migrate a log between schema versions.
    Migrate(cmd::migrate::Args),
    /// Supersede an event on a unit's log.
    Supersede(cmd::supersede::Args),
    /// Record a human approval / rejection against an event.
    Approve(cmd::approve::Args),
    /// Manage units (init / use / list / current).
    Unit {
        #[command(subcommand)]
        cmd: cmd::unit::UnitCommand,
    },
    /// Shorthand for `knotch unit current`.
    Current,
    /// Print projection summary for a unit.
    Show(cmd::show::Args),
    /// Record a phase completion or skip (skill-driven).
    Mark {
        #[command(subcommand)]
        cmd: cmd::mark::MarkCommand,
    },
    /// Record a gate decision with rationale (skill-driven).
    Gate(cmd::gate::Args),
    /// Transition the active unit to a new status (skill-driven).
    Transition(cmd::transition::Args),
    /// Claude Code hook dispatch. Reads hook JSON from stdin and
    /// maps to exit codes per `.claude/rules/hook-integration.md`.
    Hook {
        #[command(subcommand)]
        cmd: cmd::hook::HookCommand,
    },
    /// Generate shell completions.
    Completions {
        /// Target shell.
        shell: Shell,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    install_tracing(cli.quiet);

    let config = match Config::resolve(cli.root.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("knotch: {err:#}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("knotch: runtime init failed: {err}");
            return ExitCode::from(3);
        }
    };

    let out_mode = if cli.json { cmd::OutputMode::Json } else { cmd::OutputMode::Human };

    match cli.command {
        Command::Hook { cmd: hook_cmd } => runtime.block_on(cmd::hook::run(&config, hook_cmd)),
        other => {
            let result: anyhow::Result<()> = runtime.block_on(async move {
                match other {
                    Command::Init(args) => cmd::init::run(&config, out_mode, args).await,
                    Command::Log(args) => cmd::log::run(&config, out_mode, args).await,
                    Command::Reconcile(args) => cmd::reconcile::run(&config, out_mode, args).await,
                    Command::Doctor(args) => cmd::doctor::run(&config, out_mode, args).await,
                    Command::Migrate(args) => cmd::migrate::run(&config, out_mode, args).await,
                    Command::Supersede(args) => cmd::supersede::run(&config, out_mode, args).await,
                    Command::Approve(args) => cmd::approve::run(&config, out_mode, args).await,
                    Command::Unit { cmd } => cmd::unit::run(&config, out_mode, cmd).await,
                    Command::Current => {
                        cmd::unit::run(&config, out_mode, cmd::unit::UnitCommand::Current).await
                    }
                    Command::Show(args) => cmd::show::run(&config, out_mode, args).await,
                    Command::Mark { cmd } => cmd::mark::run(&config, out_mode, cmd).await,
                    Command::Gate(args) => cmd::gate::run(&config, out_mode, args).await,
                    Command::Transition(args) => {
                        cmd::transition::run(&config, out_mode, args).await
                    }
                    Command::Completions { shell } => {
                        let mut cmd_def = Cli::command();
                        let bin = cmd_def.get_name().to_owned();
                        generate(shell, &mut cmd_def, bin, &mut io::stdout());
                        Ok(())
                    }
                    Command::Hook { .. } => unreachable!("handled above"),
                }
            });
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("knotch: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn install_tracing(quiet: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(if quiet { "error" } else { "info" }));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
