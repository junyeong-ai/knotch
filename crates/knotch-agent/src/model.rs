//! Harness-side recording of between-session model switches.
//!
//! Claude Code stamps the current model name on every
//! `SessionStart` payload (startup / resume / clear / compact).
//! [`record_switch_if_changed`] is the detector used by
//! `knotch hook load-context`: on every session start it compares
//! the payload's model against `project::model_timeline`'s last
//! entry and appends a `ModelSwitched` event when they differ.
//!
//! Mid-session `/model` commands are architecturally invisible to
//! hooks — Claude Code does not re-fire `SessionStart` on `/model`
//! switches. Harnesses that do know their model at tool-call time
//! should call [`record_switch`] directly from their own
//! model-lifecycle code.

use std::path::Path;

use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, UnitId, WorkflowKind, causation::ModelId,
    event::EventBody, project::model_timeline,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    active::{ActiveUnit, resolve_active_for_hook},
    error::HookError,
    output::HookOutput,
};

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

/// SessionStart-driven detector: append `ModelSwitched` when the
/// harness's current model differs from the last model visible on
/// the unit's effective event log.
///
/// Called by `knotch hook load-context`. Silent no-op when:
///
/// - the active-unit resolver returns `NoProject` / `Uninitialized` (no log to compare
///   against);
/// - the log carries no prior `ModelSwitched` event (the first session records nothing
///   because there is no `from` value to honor the precondition against);
/// - the prior model matches `current` (no change to record).
///
/// # Errors
///
/// `Repository::load` / `Repository::append` errors surface as
/// [`HookError::Repository`].
pub async fn record_switch_if_changed<W, R>(
    project_root: &Path,
    session_id: &str,
    repo: &R,
    current: ModelId,
    causation: Causation,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize + DeserializeOwned,
{
    let unit = match resolve_active_for_hook(project_root, session_id)? {
        ActiveUnit::Active(u) => u,
        ActiveUnit::NoProject | ActiveUnit::Uninitialized => return Ok(HookOutput::Continue),
    };

    let log = repo.load(&unit).await?;
    let Some(prior) = model_timeline(&log).into_iter().next_back().map(|e| e.model) else {
        // First model event for the unit — no prior `from` to
        // anchor a `ModelSwitched` against.
        return Ok(HookOutput::Continue);
    };

    if prior == current {
        return Ok(HookOutput::Continue);
    }

    record_switch::<W, _>(repo, &unit, prior, current, causation).await
}
