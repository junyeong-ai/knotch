//! Harness-side recording of mid-session model switches.
//!
//! Claude Code has no dedicated "model switched" hook — detection
//! happens at the harness layer (comparing the current
//! `$KNOTCH_MODEL` env var to the principal's model on the last
//! recorded event, or hooking a harness-side model-change callback).
//! Per `.claude/rules/harness-decoupling.md` the kernel ships the
//! taxonomy (`EventBody::ModelSwitched`) and adopters wire their
//! own detector that invokes this helper.
//!
//! Third-party harnesses import `knotch-agent` as a library and
//! call [`record_switch`] from their model-lifecycle code. No CLI
//! wrapper ships; there is no Claude-Code-shaped trigger to bind
//! one to.

use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, UnitId, WorkflowKind,
    causation::ModelId,
    event::EventBody,
};
use serde::Serialize;

use crate::{error::HookError, output::HookOutput};

/// Append a `ModelSwitched` event against the active unit.
///
/// # No-op rejection
///
/// The kernel precondition
/// (`crates/knotch-kernel/src/event.rs::ModelSwitched`) rejects
/// `from == to`. Callers should short-circuit before invoking this
/// helper when the detected model matches the last known one.
///
/// # Errors
///
/// Any `Repository::append` failure (including
/// `PreconditionError::NoOpModelSwitch` if the caller forwarded a
/// redundant switch) surfaces as [`HookError::Repository`].
pub async fn record_switch<W, R>(
    repo: &R,
    unit: &UnitId,
    from: impl Into<ModelId>,
    to: impl Into<ModelId>,
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
        body: EventBody::ModelSwitched { from: from.into(), to: to.into() },
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::BestEffort).await?;
    Ok(HookOutput::Continue)
}
