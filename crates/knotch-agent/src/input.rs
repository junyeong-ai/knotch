//! Typed Claude Code hook stdin.
//!
//! The envelope ([`HookInput`]) carries `session_id` + `cwd` —
//! fields every hook event provides. Event-specific payloads are
//! modelled as a tagged enum ([`HookEvent`]) so consumers pattern-
//! match on the variant rather than poking optional fields at
//! runtime.

use std::path::PathBuf;

use compact_str::CompactString;
use serde::Deserialize;

/// One decoded hook invocation.
#[derive(Debug, Clone, Deserialize)]
pub struct HookInput {
    /// Claude Code session id (UUID-ish string).
    pub session_id: CompactString,
    /// Working directory at the time the hook fired.
    pub cwd: PathBuf,
    /// Event-specific payload.
    #[serde(flatten)]
    pub event: HookEvent,
}

impl HookInput {
    /// Convenience: extract `tool_input.command` when the underlying
    /// event is a Bash `PreToolUse` / `PostToolUse`.
    #[must_use]
    pub fn bash_command(&self) -> Option<&str> {
        self.event.bash_command()
    }

    /// Convenience: extract `tool_response.stdout` when the
    /// underlying event is a Bash `PostToolUse`.
    #[must_use]
    pub fn bash_response_stdout(&self) -> Option<&str> {
        self.event.bash_response_stdout()
    }
}

/// Claude Code hook event, tagged by `hook_event_name`.
///
/// Marked `#[non_exhaustive]` because the harness evolves: Claude
/// Code regularly adds new hook events (e.g. notification / stop /
/// compaction variants) and knotch should be able to surface them
/// additively without a major version bump. Downstream `match` on
/// `HookEvent` must carry a `_ => …` arm (typically a silent no-op
/// so unknown events pass through rather than failing the hook).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "hook_event_name")]
#[non_exhaustive]
pub enum HookEvent {
    /// `SessionStart` — new or resumed session.
    #[serde(rename = "SessionStart")]
    SessionStart {
        /// Matcher: `startup`, `resume`, `clear`, `compact`.
        #[serde(default)]
        source: Option<CompactString>,
    },
    /// `UserPromptSubmit` — before Claude processes a user prompt.
    #[serde(rename = "UserPromptSubmit")]
    UserPromptSubmit {
        /// Raw prompt text.
        #[serde(default)]
        prompt: Option<String>,
    },
    /// `PreToolUse` — before a tool call runs.
    #[serde(rename = "PreToolUse")]
    PreToolUse {
        /// Tool name (e.g. `Bash`, `Edit`).
        tool_name: CompactString,
        /// Tool input payload.
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
    },
    /// `PostToolUse` — after a tool call completes successfully.
    #[serde(rename = "PostToolUse")]
    PostToolUse {
        /// Tool name.
        tool_name: CompactString,
        /// Tool input payload.
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
        /// Tool response payload.
        #[serde(default)]
        tool_response: Option<serde_json::Value>,
    },
    /// `SubagentStop` — a subagent finished.
    #[serde(rename = "SubagentStop")]
    SubagentStop {
        /// Harness-assigned subagent id.
        agent_id: CompactString,
        /// Agent type (`Explore`, `Plan`, custom name).
        #[serde(default)]
        agent_type: Option<CompactString>,
        /// Absolute path to the subagent's transcript JSONL.
        #[serde(default)]
        agent_transcript_path: Option<PathBuf>,
        /// Last assistant message text.
        #[serde(default)]
        last_assistant_message: Option<String>,
        /// Set when the stop was triggered by a prior blocking
        /// stop-hook. Loop-prevention key.
        #[serde(default)]
        stop_hook_active: Option<bool>,
    },
    /// `SessionEnd` — session terminated.
    #[serde(rename = "SessionEnd")]
    SessionEnd {
        /// Matcher: `clear`, `logout`, `other`, ...
        #[serde(default)]
        reason: Option<CompactString>,
    },
}

impl HookEvent {
    /// Harness-assigned agent id when the event provides one.
    /// Currently only `SubagentStop` — other events return `None`
    /// and callers fall back to the envelope's `session_id` as a
    /// best-effort attribution.
    #[must_use]
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::SubagentStop { agent_id, .. } => Some(agent_id.as_str()),
            _ => None,
        }
    }

    /// Extract `tool_input.command` when this is a Bash
    /// `PreToolUse` / `PostToolUse`.
    #[must_use]
    pub fn bash_command(&self) -> Option<&str> {
        let (tool_name, tool_input) = match self {
            Self::PreToolUse { tool_name, tool_input }
            | Self::PostToolUse { tool_name, tool_input, .. } => (tool_name, tool_input.as_ref()?),
            _ => return None,
        };
        if tool_name.as_str() != "Bash" {
            return None;
        }
        tool_input.get("command")?.as_str()
    }

    /// Extract `tool_response.stdout` when this is a Bash
    /// `PostToolUse`.
    #[must_use]
    pub fn bash_response_stdout(&self) -> Option<&str> {
        match self {
            Self::PostToolUse { tool_name, tool_response, .. } if tool_name.as_str() == "Bash" => {
                tool_response.as_ref()?.get("stdout")?.as_str()
            }
            _ => None,
        }
    }
}
