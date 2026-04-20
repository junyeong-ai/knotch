//! `SessionEnd` lifecycle handler.
//!
//! Two responsibilities:
//!
//! 1. **Garbage-collect** the per-session active-unit pointer at
//!    `.knotch/sessions/<session_id>.toml` so the directory does not grow unboundedly.
//!    The pointer is preserved when the session is about to resume (`reason = "resume"`)
//!    so the next restart reuses the same target.
//! 2. **Surface residual queue size** so operators notice drain failures that
//!    SessionStart auto-drain could not handle.

use std::path::Path;

use crate::{active, error::HookError, output::HookOutput, queue};

/// Entry point for the `SessionEnd` hook.
///
/// `reason` mirrors Claude Code's `SessionEnd.reason` matcher —
/// `"clear"`, `"logout"`, `"prompt_input_exit"`, `"resume"`, … —
/// and governs whether the session pointer is GC'd.
pub fn finalize(
    project_root: &Path,
    session_id: &str,
    reason: Option<&str>,
) -> Result<HookOutput, HookError> {
    // 1. Surface residual queue size (advisory only).
    let queue_dir = project_root.join(".knotch").join("queue");
    if let Ok(n) = queue::queue_size(&queue_dir) {
        if n > 0 {
            tracing::info!(
                queued = n,
                "knotch: reconciler queue pending drain at session end; \
                 run `knotch reconcile` to flush"
            );
        }
    }

    // 2. GC the per-session pointer unless we're about to resume.
    if reason != Some("resume") {
        active::clear_session(project_root, session_id)?;
    }

    Ok(HookOutput::Continue)
}
