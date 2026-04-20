//! `UserPromptSubmit` refresh — re-emits the active unit context.
//!
//! Reuses [`session_start::load_context`](crate::session_start::load_context) so
//! the output format stays identical across hooks. Opt-in by default;
//! remove the hook block from `settings.json` to disable.

use std::path::Path;

use knotch_kernel::{Proposal, Repository, WorkflowKind};
use serde::de::DeserializeOwned;

use crate::{error::HookError, output::HookOutput};

/// Entry point for the `UserPromptSubmit` hook.
pub async fn refresh<W, R>(
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
    crate::session_start::load_context::<W, R>(project_root_override, cwd, session_id, repo).await
}
