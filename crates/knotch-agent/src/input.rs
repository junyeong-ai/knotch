//! Typed Claude Code hook stdin.
//!
//! The envelope ([`HookInput`]) carries `session_id`, `cwd`, and
//! the optional `agent_id` (present when a subagent is in scope).
//! Event-specific payloads are modelled as a tagged enum
//! ([`HookEvent`]) so consumers pattern-match on the variant
//! rather than poking optional fields at runtime.

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
    /// Harness-assigned agent id. Present on every event Claude
    /// Code fires inside a subagent scope (`SubagentStart`,
    /// `SubagentStop`, any tool call delegated to a subagent);
    /// `None` for main-session hooks.
    #[serde(default)]
    pub agent_id: Option<CompactString>,
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

    /// Agent id resolved from the envelope (common field) or — for
    /// `SubagentStop`, which repeats the id in its variant payload —
    /// from the event itself. Main-session hooks leave both empty
    /// and this returns `None`.
    #[must_use]
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref().or_else(|| self.event.agent_id())
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
    /// `SessionStart` — new or resumed session. Claude Code fires
    /// this at startup, after `/compact`, and after `/clear`; the
    /// `source` discriminator says which.
    #[serde(rename = "SessionStart")]
    SessionStart {
        /// Matcher: `startup`, `resume`, `clear`, `compact`.
        #[serde(default)]
        source: Option<CompactString>,
        /// Current model identifier (`claude-opus-4-7`, `claude-sonnet-4-6`, …). Claude
        /// Code stamps this on every `SessionStart` payload — `load-context` uses it to
        /// detect between-session model switches and append a matching `ModelSwitched`
        /// event when the value differs from the last one recorded.
        #[serde(default)]
        model: Option<CompactString>,
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
    /// `PostToolUse` — after a tool call **succeeds**. Failures go
    /// to [`HookEvent::PostToolUseFailure`]; consumers that want
    /// both paths match on both variants.
    #[serde(rename = "PostToolUse")]
    PostToolUse {
        /// Tool name.
        tool_name: CompactString,
        /// Harness-assigned per-call identifier (`tool_use_id` in
        /// Claude Code stdin). Unique per invocation — the
        /// `(tool, call_id)` pair is the retry-timeline key.
        #[serde(default, rename = "tool_use_id")]
        tool_use_id: Option<CompactString>,
        /// Tool input payload.
        #[serde(default)]
        tool_input: Option<serde_json::Value>,
        /// Tool response payload.
        #[serde(default)]
        tool_response: Option<serde_json::Value>,
    },
    /// `PostToolUseFailure` — Claude Code's dedicated failure hook.
    /// Fires after a tool call returns an error, with the error
    /// text already extracted; consumers no longer need to sniff
    /// `tool_response.error` on `PostToolUse`.
    #[serde(rename = "PostToolUseFailure")]
    PostToolUseFailure {
        /// Tool name.
        tool_name: CompactString,
        /// Harness-assigned per-call identifier.
        #[serde(default, rename = "tool_use_id")]
        tool_use_id: Option<CompactString>,
        /// Error text surfaced by Claude Code.
        #[serde(default)]
        error: CompactString,
        /// `true` when the user interrupted the tool (Esc / Ctrl-C)
        /// rather than the tool failing organically. The detector
        /// skips logging when this is set — user intent, not tool
        /// fault.
        #[serde(default)]
        is_interrupt: bool,
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
    /// Harness-assigned agent id when the event's own payload
    /// carries one. Currently only `SubagentStop` (which duplicates
    /// the id inside the variant). For every other event the
    /// envelope-level [`HookInput::agent_id`] is the authoritative
    /// source.
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
