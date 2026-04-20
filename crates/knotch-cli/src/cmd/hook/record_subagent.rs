//! SubagentStop → persist subagent transcript metadata.

use knotch_agent::{
    HookEvent, HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
};

use crate::config::Config;

pub(crate) async fn run(_config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let HookEvent::SubagentStop {
        agent_id,
        agent_type,
        agent_transcript_path,
        last_assistant_message,
        ..
    } = &input.event
    else {
        // Hook dispatched to the wrong subcommand; silent no-op.
        return Ok(HookOutput::Continue);
    };

    let root = project_root(&input.cwd);
    // Record only inside a knotch project; otherwise silent no-op.
    if matches!(resolve_active_for_hook(&root, input.session_id.as_str())?, ActiveUnit::NoProject) {
        return Ok(HookOutput::Continue);
    }

    Ok(knotch_agent::subagent::record(
        &root,
        agent_id.as_str(),
        agent_type.as_deref().unwrap_or("unknown"),
        agent_transcript_path.as_deref(),
        last_assistant_message.as_deref(),
    )?)
}
