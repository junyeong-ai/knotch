//! Causation and attribution.
//!
//! Every [`Event`](crate::Event) carries a [`Causation`] that records
//! *who* acted, *how* they acted, and *why*. This is the pivot that
//! makes knotch safe for AI-assisted development: session, agent id,
//! and the triggering command/hook/tool call are durable attribution
//! fields on every event.
//!
//! Three axes answer "who":
//!
//! - [`Source`] — the channel (`Cli` / `Hook` / `Observer`).
//! - `agent_id` — the subagent id, when the action was driven by an LLM agent (`None` for
//!   CLI operators, observers, and the main session where Claude Code doesn't surface a
//!   distinct id).
//! - [`SessionId`] — the conversation / run scope.
//!
//! Model attribution lives on [`EventBody::ModelSwitched`](crate::event::EventBody)
//! events — not on every event's causation — because the model can change within a
//! session. Callers who need "which model produced event X" read the effective
//! [`model_timeline`](crate::project::model_timeline) up to that event.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Attribution chain for a single [`Event`](crate::Event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Causation {
    /// Channel the action arrived through.
    pub source: Source,
    /// Conversation / run scope; `None` for one-off CLI invocations
    /// that aren't threaded into a session.
    pub session: Option<SessionId>,
    /// Subagent id; `None` for CLI operators, observer-driven events,
    /// and the main session (where Claude Code doesn't expose a
    /// distinct agent id beyond the session).
    pub agent_id: Option<AgentId>,
    /// Specific trigger within the `source`.
    pub trigger: Trigger,
}

impl Causation {
    /// Convenience constructor for the common "CLI subcommand" case.
    #[must_use]
    pub fn cli(command: impl Into<CompactString>) -> Self {
        Self::new(Source::Cli, Trigger::Command { name: command.into() })
    }

    /// Full-fat constructor — exposed to peer crates because `Causation`
    /// is `#[non_exhaustive]` and therefore cannot be built with a
    /// struct literal outside this crate.
    #[must_use]
    pub fn new(source: Source, trigger: Trigger) -> Self {
        Self { source, session: None, agent_id: None, trigger }
    }

    /// Attach a session id.
    #[must_use]
    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session = Some(session);
        self
    }

    /// Attach an agent id. Use when the event is driven by a
    /// named subagent (e.g. Claude Code's `SubagentStop` payload).
    #[must_use]
    pub fn with_agent_id(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }
}

/// Channel an action arrived through.
///
/// Three variants cover every legitimate knotch write path. All
/// other nuance (who / what / why) lives in [`Causation::agent_id`]
/// and [`Trigger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Source {
    /// Interactive CLI invocation (human operator or agent-driven
    /// subprocess — the author distinction lives on `agent_id`).
    Cli,
    /// Claude Code hook dispatch (every variant of `HookEvent`).
    Hook,
    /// Reconciler observer pass — the observer name lives in
    /// [`Trigger::Observer`].
    Observer,
}

/// Specific trigger within a `Source`.
///
/// Data-carrying variants are **struct variants** (even when they
/// would be natural as newtypes) — `#[serde(tag = "kind")]` does not
/// support the `(tag, newtype + primitive)` combination under
/// `serde_jcs` (RFC 8785 canonical JSON), and fingerprint stability
/// depends on JCS round-tripping every field of every proposal. Unit
/// variants are fine: they serialize as a single-key
/// `{"kind":"..."}` object regardless of backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Trigger {
    /// Named CLI command or shell invocation.
    Command {
        /// Command name (e.g. `init`, `mark`).
        name: CompactString,
    },
    /// Git hook name.
    GitHook {
        /// Hook subcommand (e.g. `check-commit`, `verify-commit`).
        name: CompactString,
    },
    /// Agent tool call. `tool` is the tool name; `call_id` is the
    /// invocation identifier from the agent harness.
    ToolInvocation {
        /// Tool name (e.g. `bash`, `edit_file`).
        tool: CompactString,
        /// Harness-assigned invocation id.
        call_id: CompactString,
    },
    /// Proposed by a reconciler observer; the string is the observer
    /// name from `Observer::name`.
    Observer {
        /// Observer name as returned by `Observer::name`.
        name: CompactString,
    },
}

/// Conversation / run scope identifier.
///
/// Preferred form is an RFC-9562 UUIDv7 — time-sortable and
/// OpenTelemetry-compatible. Harnesses that supply free-form session
/// ids (e.g. Claude Code sometimes returns short strings) map to
/// [`SessionId::Opaque`]; [`SessionId::as_otel_bytes`] hashes those
/// to a deterministic 128-bit value for downstream OTel export.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionId {
    /// UUID-formatted session id. Serializes as a UUID string.
    Uuid(uuid::Uuid),
    /// Opaque (non-UUID) session id. Serializes as a bare string.
    Opaque(CompactString),
}

impl SessionId {
    /// Generate a fresh v7 session id.
    #[must_use]
    pub fn new_v7() -> Self {
        Self::Uuid(uuid::Uuid::now_v7())
    }

    /// Wrap an opaque string id (harness-specific format).
    ///
    /// Kept as a named constructor rather than `impl From<String>`
    /// because `SessionId` has two variants — the named form keeps
    /// call sites explicit about which variant they mean.
    #[must_use]
    pub fn opaque(s: impl Into<CompactString>) -> Self {
        Self::Opaque(s.into())
    }

    /// Parse a session id from a string. Tries UUID first, falls
    /// back to [`SessionId::Opaque`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        uuid::Uuid::parse_str(s).map(Self::Uuid).unwrap_or_else(|_| Self::Opaque(s.into()))
    }

    /// Deterministic 128-bit representation for OpenTelemetry
    /// export. UUID variants return the UUID bytes directly; opaque
    /// variants return the first 16 bytes of BLAKE3 over the string.
    #[must_use]
    pub fn as_otel_bytes(&self) -> [u8; 16] {
        match self {
            Self::Uuid(u) => *u.as_bytes(),
            Self::Opaque(s) => {
                let hash = blake3::hash(s.as_bytes());
                let mut out = [0u8; 16];
                out.copy_from_slice(&hash.as_bytes()[..16]);
                out
            }
        }
    }
}

/// Wrap a UUID as a `SessionId`. Selects the `Uuid` variant.
impl From<uuid::Uuid> for SessionId {
    fn from(u: uuid::Uuid) -> Self {
        Self::Uuid(u)
    }
}

/// Agent instance identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub CompactString);

impl From<CompactString> for AgentId {
    fn from(s: CompactString) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(CompactString::from(s))
    }
}

impl AgentId {
    /// Access the inner slug.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl core::fmt::Display for AgentId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// Model identifier (e.g. `claude-opus-4-7`).
///
/// Populated on [`EventBody::ModelSwitched`](crate::event::EventBody) events; the
/// current model at time `t` is derived from the effective
/// [`model_timeline`](crate::project::model_timeline).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(pub CompactString);

impl core::fmt::Display for ModelId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}
