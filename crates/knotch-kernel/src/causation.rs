//! Causation and attribution.
//!
//! Every [`Event`](crate::Event) carries a [`Causation`] that records
//! *who* acted, *how* they acted, and *why*. This is the pivot that
//! makes knotch safe for AI-assisted development (principle 8, RFC
//! 0001): sessions, traces, costs, and agent identities are typed.
//!
//! Sensitive fields (`Person`, `AgentId`) are marked with
//! `#[derive(Sensitive)]` so tracing subscribers hash them by default.
//! The derive lives in `knotch-derive`; the marker trait is [`Sensitive`].

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use crate::id::EventId;

/// Marker trait for types whose `Debug`/`Serialize` output is subject
/// to PII redaction in tracing contexts.
///
/// Implemented by the `#[derive(Sensitive)]` macro in `knotch-derive`
/// and honored by tracing attribute serializers.
pub trait Sensitive {}

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
    /// OpenTelemetry-compatible 128-bit trace identifier.
    pub trace: Option<TraceId>,
    /// Specific trigger within the `source`.
    pub trigger: Trigger,
    /// Event-graph linkage — the prior event that caused this one.
    pub parent_event: Option<EventId>,
}

impl Causation {
    /// Convenience constructor for the common "CLI / human / manual" case.
    #[must_use]
    pub fn cli(command: impl Into<CompactString>) -> Self {
        Self {
            source: Source::Cli,
            principal: Principal::System { service: CompactString::from("cli") },
            session: None,
            trace: None,
            trigger: Trigger::Command { name: command.into() },
            parent_event: None,
        }
    }

    /// Full-fat constructor — exposed to peer crates because `Causation`
    /// is `#[non_exhaustive]` and therefore cannot be built with a
    /// struct literal outside this crate.
    #[must_use]
    pub fn new(source: Source, principal: Principal, trigger: Trigger) -> Self {
        Self { source, principal, session: None, trace: None, trigger, parent_event: None }
    }

    /// Attach a session id.
    #[must_use]
    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session = Some(session);
        self
    }

    /// Attach a trace id.
    #[must_use]
    pub fn with_trace(mut self, trace: TraceId) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Attach a parent-event id.
    #[must_use]
    pub fn with_parent_event(mut self, parent: EventId) -> Self {
        self.parent_event = Some(parent);
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
    /// A human operator.
    Human {
        /// Identity of the operator (sensitive).
        person: Person,
    },
    /// An AI agent.
    Agent {
        /// Agent instance id (sensitive).
        agent_id: AgentId,
        /// Model identifier (public — e.g. `claude-opus-4-7`).
        model: ModelId,
        /// Harness identifier (public — e.g. `claude-code/1.0`).
        harness: Harness,
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
/// variants (e.g. `Manual`) are fine: they serialize as a single-key
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

/// OpenTelemetry-compatible 128-bit trace identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Return the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Wrap raw trace bytes — typically the 16-byte big-endian form
/// from an OTel span context.
impl From<[u8; 16]> for TraceId {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

/// Sensitive operator identity. The `#[derive(Sensitive)]` in
/// `knotch-derive` wraps this with tracing-redacted Debug/Serialize.
/// Until the derive lands we implement [`Sensitive`] manually.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Person(pub CompactString);

impl Sensitive for Person {}

impl core::fmt::Display for Person {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// Sensitive agent instance identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub CompactString);

impl Sensitive for AgentId {}

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

/// Non-sensitive model identifier (e.g. `claude-opus-4-7`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(pub CompactString);

impl core::fmt::Display for ModelId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// Non-sensitive harness identifier (e.g. `claude-code/1.0`,
/// `cursor/0.45`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Harness(pub CompactString);

impl core::fmt::Display for Harness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

