//! SessionEnd → surface queue size + GC per-session pointer.

use knotch_agent::{HookEvent, HookInput, HookOutput, active::project_root};

use crate::config::Config;

pub(crate) async fn run(_config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let reason = match &input.event {
        HookEvent::SessionEnd { reason } => reason.as_deref(),
        _ => None,
    };
    let root = project_root(&input.cwd);
    Ok(knotch_agent::session_end::finalize(
        &root,
        input.session_id.as_str(),
        reason,
    )?)
}
