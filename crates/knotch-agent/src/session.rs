//! `SessionStart` hook — injects active-unit context.
//!
//! Resolves the active unit via the three-layer chain in
//! [`crate::active`] (env → session → global). When a unit is
//! active, snapshots it into the per-session pointer so subsequent
//! hooks in the same session are stable against project-global
//! reassignments.

use std::path::Path;

use knotch_kernel::{Proposal, Repository, UnitId, WorkflowKind};
use serde::de::DeserializeOwned;

use crate::{
    active::{ActiveUnit, project_root, resolve_active_for_hook, write_active_for_session},
    error::HookError,
    output::HookOutput,
};

/// Run the `SessionStart` hook.
///
/// `project_root_override` is optional: callers may pass an explicit
/// root (for tests), otherwise the ancestor chain from `cwd` is
/// searched for `knotch.toml`.
///
/// `session_id` is required so the pointer resolution can honor the
/// per-session layer; pass the value from `HookInput::session_id`
/// directly.
pub async fn load_context<W, R>(
    project_root_override: Option<&Path>,
    cwd: &Path,
    session_id: &str,
    repo: &R,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    R: Repository<W>,
    Proposal<W>: DeserializeOwned,
{
    let root = project_root_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_root(cwd));

    let active = resolve_active_for_hook(&root, session_id)?;

    // Snapshot the resolved unit into the per-session pointer so
    // later hooks see a stable target even if `knotch unit use` runs
    // elsewhere mid-session.
    if let ActiveUnit::Active(unit) = &active {
        write_active_for_session(&root, Some(unit), session_id, "session-start")?;
    }

    // Auto-drain reconciler queue. Failures are logged, not
    // surfaced — the context injection must not depend on drain
    // success.
    let queue_dir = root.join(".knotch").join("queue");
    match crate::queue::drain::<W, R>(&queue_dir, repo).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(drained = n, "knotch: reconciler queue drained at session start"),
        Err(err) => tracing::warn!("knotch: queue drain failed: {err}"),
    }

    match active {
        ActiveUnit::NoProject => Ok(HookOutput::Continue),
        ActiveUnit::Uninitialized => Ok(HookOutput::context(
            "knotch: project initialized but no active unit. \
             Run `knotch unit use <id>` to target one."
                .to_owned(),
        )),
        ActiveUnit::Active(unit) => {
            let log = repo.load(&unit).await?;
            let phase = knotch_kernel::project::current_phase(repo.workflow(), &log);
            let status = knotch_kernel::project::current_status(&log);
            let shipped = knotch_kernel::project::shipped_milestones(&log);
            let ctx = format_context::<W>(&unit, phase.as_ref(), status.as_ref(), shipped.len());
            Ok(HookOutput::Context(ctx))
        }
    }
}

fn format_context<W: WorkflowKind>(
    unit: &UnitId,
    phase: Option<&W::Phase>,
    status: Option<&knotch_kernel::StatusId>,
    shipped_count: usize,
) -> String {
    let phase_str: std::borrow::Cow<'_, str> = phase
        .map(knotch_kernel::PhaseKind::id)
        .unwrap_or(std::borrow::Cow::Borrowed("(none)"));
    let status_str = status.map(knotch_kernel::StatusId::as_str).unwrap_or("(none)");
    format!(
        "knotch:\n  active unit: {}\n  current phase: {phase_str}\n  current status: {status_str}\n  shipped milestones: {shipped_count}\n  emit events via: /knotch-mark, /knotch-gate, /knotch-transition",
        unit.as_str()
    )
}
