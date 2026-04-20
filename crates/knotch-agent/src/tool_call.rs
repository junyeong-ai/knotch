//! Harness-side recording of tool-call failures.
//!
//! Claude Code's `PostToolUse` hook fires on every tool call,
//! successful or not. Deciding whether a given invocation was a
//! failure — parsing a non-zero exit, a rate-limit JSON error, a
//! timeout — is inherently harness-specific (Claude Code shapes it
//! differently from Cursor or Aider). Per
//! `.claude/rules/harness-decoupling.md` the kernel ships the
//! shared taxonomy (`EventBody::ToolCallFailed` +
//! `FailureReason`), and adopters wire their own failure detector
//! that invokes this helper.
//!
//! Third-party harnesses therefore import `knotch-agent` as a
//! library and call [`record_failure`] directly from their
//! failure-handling path. No CLI wrapper ships — one would have to
//! hard-code a Claude-Code-shaped tool-response parser, which
//! `harness-decoupling.md` forbids.

use compact_str::CompactString;
use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, UnitId, WorkflowKind,
    event::{EventBody, FailureReason},
};
use serde::Serialize;

use crate::{error::HookError, output::HookOutput};

/// Append a `ToolCallFailed` event against the active unit.
///
/// # Monotonic attempts
///
/// The caller owns `attempt` numbering — per the kernel
/// precondition (`crates/knotch-kernel/src/event.rs::ToolCallFailed`)
/// the `(tool, call_id, attempt)` tuple must be strictly monotonic
/// across the unit's log for a given `(tool, call_id)` pair.
/// Harnesses typically mint `attempt = prior_max + 1` after
/// consulting [`project::tool_call_timeline`].
///
/// # Errors
///
/// Any `Repository::append` failure (including
/// `PreconditionError::NonMonotonicAttempt`) surfaces as
/// [`HookError::Repository`]. Post-failure retry / queue handling
/// is the caller's responsibility (see
/// `.claude/rules/hook-integration.md`).
///
/// [`project::tool_call_timeline`]:
///     knotch_kernel::project::tool_call_timeline
pub async fn record_failure<W, R>(
    repo: &R,
    unit: &UnitId,
    tool: impl Into<CompactString>,
    call_id: impl Into<CompactString>,
    attempt: core::num::NonZeroU32,
    reason: FailureReason,
    causation: Causation,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::ToolCallFailed {
            tool: tool.into(),
            call_id: call_id.into(),
            attempt,
            reason,
        },
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::BestEffort).await?;
    Ok(HookOutput::Continue)
}
