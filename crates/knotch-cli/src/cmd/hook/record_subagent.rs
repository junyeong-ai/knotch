//! SubagentStop → append `EventBody::SubagentCompleted` against the
//! active unit. The event lands under the active unit and is
//! reachable via `knotch-query`. Hooks with no active unit silent-
//! noop — subagent delegation outside a tracked unit is not a
//! knotch concern.

use compact_str::CompactString;
use knotch_agent::{
    HookEvent, HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
    causation::hook_causation,
};
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

/// Claude Code can emit very long `last_assistant_message` strings
/// (~256 KiB). The ledger keeps an 8 KiB cap and points at the
/// transcript file for the rest — the JSONL log is not a place to
/// store free-form transcript bodies.
const MAX_LAST_MESSAGE_BYTES: usize = 8 * 1024;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
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
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        // No knotch project, or project with no active unit —
        // either way there is no log to append to; subagent is
        // silently unrecorded.
        ActiveUnit::NoProject | ActiveUnit::Uninitialized => return Ok(HookOutput::Continue),
    };

    let causation = hook_causation(&input, "record-subagent");
    let repo = config.build_repository()?;
    let last_message_capped = last_assistant_message.as_deref().map(cap_last_message);
    Ok(knotch_agent::subagent::record::<ConfigWorkflow, _>(
        &repo,
        &unit,
        agent_id.clone(),
        agent_type.clone(),
        agent_transcript_path.as_deref(),
        last_message_capped,
        causation,
    )
    .await?)
}

fn cap_last_message(m: &str) -> CompactString {
    if m.len() <= MAX_LAST_MESSAGE_BYTES {
        return CompactString::from(m);
    }
    // Walk back from the byte cap until we land on a UTF-8 boundary
    // so we never split a multi-byte codepoint.
    let mut cut = MAX_LAST_MESSAGE_BYTES;
    while cut > 0 && !m.is_char_boundary(cut) {
        cut -= 1;
    }
    let truncated = &m[..cut];
    CompactString::from(format!("{truncated}… [truncated, {} bytes elided]", m.len() - cut,))
}
