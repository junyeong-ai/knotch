//! Causation and attribution.
//!
//! Every [`Event`](crate::Event) carries a [`Causation`] that records
//! *who* acted, *how* they acted, and *why*. This is the pivot that
//! makes knotch safe for AI-assisted development (principle 8, RFC
//! 0001): session id and typed agent/model identities are the
//! durable attribution surface.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Attribution chain for a single [`Event`](crate::Event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Causation {
    /// Channel the action arrived through.
    pub source: Source,
    /// Identity of the actor.
    pub principal: Principal,
    /// Conversation / run scope; `None` for ad-hoc one-off actions.
    pub session: Option<SessionId>,
    /// Specific trigger within the `source`.
    pub trigger: Trigger,
}

impl Causation {
    /// Convenience constructor for the common "CLI / human / manual" case.
    #[must_use]
    pub fn cli(command: impl Into<CompactString>) -> Self {
        Self {
            source: Source::Cli,
            principal: Principal::System { service: CompactString::from("cli") },
            session: None,
            trigger: Trigger::Command { name: command.into() },
        }
    }

    /// Full-fat constructor — exposed to peer crates because `Causation`
    /// is `#[non_exhaustive]` and therefore cannot be built with a
    /// struct literal outside this crate.
    #[must_use]
    pub fn new(source: Source, principal: Principal, trigger: Trigger) -> Self {
        Self { source, principal, session: None, trigger }
    }

    /// Attach a session id.
    #[must_use]
    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session = Some(session);
        self
    }
}

/// Channel an action arrived through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Source {
    /// Interactive CLI invocation.
    Cli,
    /// Git hook (pre-commit, post-commit, etc.).
    Hook,
    /// Human operator via any UI.
    User,
    /// Automated test.
    Test,
    /// AI agent.
    Agent,
    /// Anything else, named by the embedder.
    External(CompactString),
}

/// Who performed the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Principal {
    /// An AI agent.
    Agent {
        /// Agent instance id.
        agent_id: AgentId,
        /// Model identifier (e.g. `claude-opus-4-7`).
        model: ModelId,
    },
    /// A background service or automated system.
    System {
        /// Named service (e.g. `reconciler`, `cli`, `ci`).
        service: CompactString,
    },
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(pub CompactString);

impl core::fmt::Display for ModelId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}
