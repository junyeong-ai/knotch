//! `knotch hook <subcommand>` — Claude Code hook entry points.
//!
//! Every subcommand reads one JSON document from stdin (the hook
//! event payload), dispatches to the corresponding `knotch-agent`
//! function, and maps the result to stdout + exit code per the
//! contract in `.claude/rules/hook-integration.md`.
//!
//! ## Exit-code policy
//!
//! The subcommand's [`HookEventKind`] governs error handling:
//!
//! | Event family   | stdin parse fail | agent fn fail | Policy |
//! |----------------|------------------|---------------|--------|
//! | `Blocking`     | exit **2**       | exit **2**    | Ledger integrity requires blocking on any failure. |
//! | `NonBlocking`  | exit **0**       | exit **0**    | Post-action hooks cannot undo; errors log only. |

pub(crate) mod check_commit;
pub(crate) mod finalize_session;
pub(crate) mod guard_rewrite;
pub(crate) mod load_context;
pub(crate) mod record_revert;
pub(crate) mod record_subagent;
pub(crate) mod record_tool_failure;
pub(crate) mod refresh_context;
pub(crate) mod verify_commit;

use std::process::ExitCode;

use clap::Subcommand;
use knotch_agent::{HookInput, HookOutput};
use tokio::io::AsyncReadExt as _;

use crate::config::Config;

/// Hook subcommand enum. All matcher + `if` filtering lives in
/// `.claude/settings.json`; the CLI only receives the dispatched
/// event on stdin.
#[derive(Debug, Subcommand)]
pub(crate) enum HookCommand {
    /// Inject active-unit context (SessionStart).
    LoadContext,
    /// Refresh active-unit context (UserPromptSubmit — optional).
    RefreshContext,
    /// Validate a pending git commit (PreToolUse).
    CheckCommit,
    /// Record `MilestoneShipped` after git commit (PostToolUse).
    VerifyCommit,
    /// Record `MilestoneReverted` after git revert (PostToolUse).
    RecordRevert,
    /// Block history-rewriting git operations (PreToolUse) — force
    /// push, reset --hard, branch -D, checkout --, clean -f.
    GuardRewrite,
    /// Record a subagent transcript (SubagentStop).
    RecordSubagent,
    /// Record `ToolCallFailed` when Claude Code fires
    /// `PostToolUseFailure`.
    RecordToolFailure,
    /// Surface reconciler-queue size at session end (SessionEnd).
    FinalizeSession,
}

/// Whether the hook event can be blocked via exit 2.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HookEventKind {
    /// `PreToolUse` — exit 2 on any failure. The tool call is blocked
    /// until the underlying issue is resolved.
    Blocking,
    /// `PostToolUse` / `SessionStart` / `SessionEnd` / `SubagentStop`
    /// / `UserPromptSubmit` — exit 0 on failure. Claude Code treats
    /// other non-zero codes as non-blocking, so we never emit them.
    NonBlocking,
}

impl HookCommand {
    /// Claude Code event name (matches the on-disk settings key).
    fn event_name(&self) -> &'static str {
        match self {
            Self::LoadContext => "SessionStart",
            Self::RefreshContext => "UserPromptSubmit",
            Self::CheckCommit | Self::GuardRewrite => "PreToolUse",

            Self::VerifyCommit | Self::RecordRevert => "PostToolUse",
            Self::RecordToolFailure => "PostToolUseFailure",
            Self::RecordSubagent => "SubagentStop",
            Self::FinalizeSession => "SessionEnd",
        }
    }

    /// Blocking / NonBlocking policy for this subcommand.
    fn kind(&self) -> HookEventKind {
        match self {
            Self::CheckCommit | Self::GuardRewrite => HookEventKind::Blocking,

            _ => HookEventKind::NonBlocking,
        }
    }
}

/// Top-level dispatch used by `main.rs`.
pub(crate) async fn run(config: &Config, cmd: HookCommand) -> ExitCode {
    // Compute kind *before* reading stdin so a parse failure still
    // maps to the correct exit code.
    let event_name = cmd.event_name();
    let kind = cmd.kind();

    let input = match read_input().await {
        Ok(i) => i,
        Err(e) => {
            return emit_error(&anyhow::anyhow!("stdin parse failed: {e}"), kind);
        }
    };

    let result = match cmd {
        HookCommand::LoadContext => load_context::run(config, input).await,
        HookCommand::RefreshContext => refresh_context::run(config, input).await,
        HookCommand::CheckCommit => check_commit::run(config, input).await,
        HookCommand::VerifyCommit => verify_commit::run(config, input).await,
        HookCommand::RecordRevert => record_revert::run(config, input).await,
        HookCommand::GuardRewrite => guard_rewrite::run(config, input).await,
        HookCommand::RecordSubagent => record_subagent::run(config, input).await,
        HookCommand::RecordToolFailure => record_tool_failure::run(config, input).await,
        HookCommand::FinalizeSession => finalize_session::run(config, input).await,
    };

    match result {
        Ok(out) => emit(out, event_name),
        Err(e) => emit_error(&e, kind),
    }
}

async fn read_input() -> anyhow::Result<HookInput> {
    let mut buf = String::new();
    tokio::io::stdin().read_to_string(&mut buf).await?;
    let input: HookInput = serde_json::from_str(&buf)?;
    Ok(input)
}

fn emit(out: HookOutput, event_name: &str) -> ExitCode {
    match out {
        HookOutput::Continue => ExitCode::SUCCESS,
        HookOutput::Context(ctx) => {
            let value = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": event_name,
                    "additionalContext": ctx,
                }
            });
            println!("{value}");
            ExitCode::SUCCESS
        }
        HookOutput::Block { reason } => {
            eprintln!("{reason}");
            ExitCode::from(2)
        }
        HookOutput::UpdateInput(updated) => {
            let value = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": event_name,
                    "permissionDecision": "allow",
                    "updatedInput": updated,
                }
            });
            println!("{value}");
            ExitCode::SUCCESS
        }
        HookOutput::Ask { reason } => {
            let value = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": event_name,
                    "permissionDecision": "ask",
                    "permissionDecisionReason": reason,
                }
            });
            println!("{value}");
            ExitCode::SUCCESS
        }
    }
}

fn emit_error(err: &anyhow::Error, kind: HookEventKind) -> ExitCode {
    match kind {
        HookEventKind::Blocking => {
            eprintln!("knotch hook error: {err:#}");
            ExitCode::from(2)
        }
        HookEventKind::NonBlocking => {
            tracing::warn!("knotch hook error: {err:#}");
            ExitCode::SUCCESS
        }
    }
}
