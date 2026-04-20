//! PostToolUseFailure → append `EventBody::ToolCallFailed`.
//!
//! Claude Code fires a dedicated `PostToolUseFailure` hook with
//! the error text already extracted. This subcommand subscribes
//! to that event and appends a `ToolCallFailed` entry against the
//! active unit.
//!
//! ## Filtering
//!
//! - `is_interrupt = true` — user hit Esc / Ctrl-C. That's a
//!   choice, not a failure; we no-op so the retry timeline does
//!   not inflate with intentional cancellations.
//! - Missing `tool_use_id` — without a stable per-call id the
//!   `(tool, call_id)` monotonicity precondition cannot hold, so
//!   we log and drop rather than synthesising a placeholder.

use knotch_agent::{
    HookEvent, HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
    causation::hook_causation,
};
use knotch_kernel::event::ToolCallFailureReason;
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let HookEvent::PostToolUseFailure { tool_name, tool_use_id, error, is_interrupt } =
        &input.event
    else {
        return Ok(HookOutput::Continue);
    };

    if *is_interrupt {
        // User cancellation — not a tool failure.
        return Ok(HookOutput::Continue);
    }

    let Some(call_id) = tool_use_id.clone() else {
        tracing::warn!(
            tool = %tool_name,
            "record-tool-failure: missing tool_use_id, dropping",
        );
        return Ok(HookOutput::Continue);
    };

    let root = project_root(&input.cwd);
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        ActiveUnit::NoProject | ActiveUnit::Uninitialized => return Ok(HookOutput::Continue),
    };

    let causation = hook_causation(&input, "record-tool-failure");
    let repo = config.build_repository()?;
    // Every Claude Code tool invocation produces a distinct
    // `tool_use_id`, so the (tool, call_id) pair never repeats and
    // attempt = 1 is correct. Harnesses that retry under a shared
    // call_id call `knotch_agent::tool_call::record_failure` directly
    // with their own monotonic attempt counter.
    let attempt = core::num::NonZeroU32::new(1).unwrap();
    let reason = ToolCallFailureReason::Backend { message: cap_message(error) };
    Ok(knotch_agent::tool_call::record_failure::<ConfigWorkflow, _>(
        &repo,
        &unit,
        tool_name.clone(),
        call_id,
        attempt,
        reason,
        causation,
    )
    .await?)
}

/// Cap error messages at 1 KiB so the ledger never stores a
/// rambling tool backtrace. The message is operator-facing; the
/// full detail lives in the tool's own transcript.
const MAX_MESSAGE_BYTES: usize = 1024;

fn cap_message(raw: &str) -> compact_str::CompactString {
    if raw.len() <= MAX_MESSAGE_BYTES {
        return compact_str::CompactString::from(raw);
    }
    let mut cut = MAX_MESSAGE_BYTES;
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    compact_str::CompactString::from(format!("{}… [{} bytes elided]", &raw[..cut], raw.len() - cut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_message_preserves_short_strings() {
        assert_eq!(cap_message("file not found").as_str(), "file not found");
    }

    #[test]
    fn cap_message_truncates_at_utf8_boundary() {
        let long = "가".repeat(1000);
        let capped = cap_message(&long);
        assert!(capped.len() < long.len() + 50);
        assert!(capped.contains("…"));
    }
}
