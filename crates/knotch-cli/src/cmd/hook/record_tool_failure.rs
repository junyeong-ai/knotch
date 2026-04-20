//! PostToolUse → append `EventBody::ToolCallFailed` when the
//! `tool_response` carries a failure signal.
//!
//! Fires on every tool call but only appends when a failure is
//! detected. The detector is deliberately **conservative** — it
//! reports nothing for ambiguous signals so a no-op PostToolUse
//! stays a silent pass-through. A false positive would inflate
//! the log with spurious ToolCallFailed events and mis-color the
//! retry timeline.
//!
//! ## Detection rule
//!
//! Claude Code PostToolUse payloads converge on two error surfaces:
//!
//! 1. Any tool: a non-empty string at `tool_response.error`. Observed for Edit, Write,
//!    Read, Grep, Glob, WebFetch, WebSearch, and the MCP call family.
//! 2. Bash specifically: a non-zero `exit_code` on `tool_response`, with the stderr text
//!    appended to the reason for operator context.
//!
//! Neither the rate-limit nor the timeout variant of
//! `ToolCallFailureReason` is emitted from this detector — both
//! require exact duration values we cannot extract from the hook
//! payload alone. Harnesses that wrap the LLM backend directly
//! have that timing information and should build a richer
//! classifier against `knotch_agent::tool_call::record_failure`.

use compact_str::CompactString;
use knotch_agent::{
    HookEvent, HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
    causation::hook_causation,
};
use knotch_kernel::event::ToolCallFailureReason;
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let HookEvent::PostToolUse { tool_name, tool_use_id, tool_response, .. } = &input.event else {
        return Ok(HookOutput::Continue);
    };

    let Some(reason) = classify(tool_name, tool_response.as_ref()) else {
        return Ok(HookOutput::Continue);
    };

    let Some(call_id) = tool_use_id.clone() else {
        // Claude Code stopped sending the id — without a stable
        // `(tool, call_id)` pair we can't honor the monotonic
        // precondition. Log and drop.
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

/// Classify a PostToolUse payload into a
/// [`ToolCallFailureReason`]. Returns `None` when the response
/// looks successful.
///
/// Public at the module boundary so unit tests can exercise the
/// detector without constructing a full hook invocation.
pub(crate) fn classify(
    tool_name: &CompactString,
    tool_response: Option<&serde_json::Value>,
) -> Option<ToolCallFailureReason> {
    let response = tool_response?;

    // Universal signal — every Claude Code tool surfaces failures
    // via a top-level `error` string.
    if let Some(error) = response.get("error").and_then(|v| v.as_str())
        && !error.is_empty()
    {
        return Some(ToolCallFailureReason::Backend { message: cap_message(error) });
    }

    // Bash-specific: non-zero `exit_code`. The `error` field is
    // typically absent on non-zero exits, so this is the primary
    // failure signal for that tool.
    if tool_name.as_str() == "Bash"
        && let Some(exit_code) = response.get("exit_code").and_then(serde_json::Value::as_i64)
        && exit_code != 0
    {
        let stderr = response.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
        let msg = if stderr.is_empty() {
            format!("exit {exit_code}")
        } else {
            format!("exit {exit_code}: {stderr}")
        };
        return Some(ToolCallFailureReason::Backend { message: cap_message(&msg) });
    }

    None
}

/// Cap error messages at 1 KiB so the ledger never stores a
/// rambling tool backtrace. The message is operator-facing; the
/// full detail lives in the tool's own transcript.
const MAX_MESSAGE_BYTES: usize = 1024;

fn cap_message(raw: &str) -> CompactString {
    if raw.len() <= MAX_MESSAGE_BYTES {
        return CompactString::from(raw);
    }
    let mut cut = MAX_MESSAGE_BYTES;
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    CompactString::from(format!("{}… [{} bytes elided]", &raw[..cut], raw.len() - cut))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact(s: &str) -> CompactString {
        CompactString::from(s)
    }

    #[test]
    fn classify_returns_none_for_successful_response() {
        let response = serde_json::json!({ "stdout": "ok" });
        assert!(classify(&compact("Read"), Some(&response)).is_none());
    }

    #[test]
    fn classify_returns_none_when_response_is_missing() {
        assert!(classify(&compact("Read"), None).is_none());
    }

    #[test]
    fn classify_detects_error_field() {
        let response = serde_json::json!({ "error": "file not found" });
        let reason = classify(&compact("Read"), Some(&response)).expect("classified");
        assert!(matches!(
            reason,
            ToolCallFailureReason::Backend { message } if message.as_str() == "file not found"
        ));
    }

    #[test]
    fn classify_ignores_empty_error_field() {
        let response = serde_json::json!({ "error": "" });
        assert!(classify(&compact("Read"), Some(&response)).is_none());
    }

    #[test]
    fn classify_bash_nonzero_exit_without_error_field() {
        let response = serde_json::json!({ "exit_code": 127, "stderr": "command not found" });
        let reason = classify(&compact("Bash"), Some(&response)).expect("classified");
        assert!(matches!(
            reason,
            ToolCallFailureReason::Backend { message } if message.as_str() == "exit 127: command not found"
        ));
    }

    #[test]
    fn classify_bash_zero_exit_passes_through() {
        let response = serde_json::json!({ "exit_code": 0, "stdout": "ok" });
        assert!(classify(&compact("Bash"), Some(&response)).is_none());
    }

    #[test]
    fn classify_non_bash_ignores_exit_code_field() {
        // Hypothetical tool response carrying `exit_code` but no
        // `error`. Only Bash gets the non-zero-exit fast-path.
        let response = serde_json::json!({ "exit_code": 2 });
        assert!(classify(&compact("Read"), Some(&response)).is_none());
    }

    #[test]
    fn cap_message_truncates_at_utf8_boundary() {
        let long = "가".repeat(1000);
        let capped = cap_message(&long);
        assert!(capped.len() < long.len() + 50);
        assert!(capped.contains("…"));
    }
}
