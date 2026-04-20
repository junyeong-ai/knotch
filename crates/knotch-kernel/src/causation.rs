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
use rust_decimal::Decimal;
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
    /// LLM cost attribution.
    pub cost: Option<Cost>,
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
            cost: None,
        }
    }

    /// Full-fat constructor — exposed to peer crates because `Causation`
    /// is `#[non_exhaustive]` and therefore cannot be built with a
    /// struct literal outside this crate.
    #[must_use]
    pub fn new(source: Source, principal: Principal, trigger: Trigger) -> Self {
        Self {
            source,
            principal,
            session: None,
            trace: None,
            trigger,
            parent_event: None,
            cost: None,
        }
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

    /// Attach a cost.
    #[must_use]
    pub fn with_cost(mut self, cost: Cost) -> Self {
        self.cost = Some(cost);
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
    /// User-initiated without a more specific channel.
    Manual,
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

/// Cost attribution for a single event caused by an AI agent.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cost {
    /// USD cost with fixed-point precision.
    pub usd: Option<Decimal>,
    /// Prompt / input tokens consumed.
    pub tokens_in: u32,
    /// Completion / output tokens produced.
    pub tokens_out: u32,
}

impl Cost {
    /// Build a `Cost`. `non_exhaustive` prevents struct-literal
    /// construction from peer crates, so expose a dedicated
    /// constructor.
    #[must_use]
    pub fn new(usd: Option<Decimal>, tokens_in: u32, tokens_out: u32) -> Self {
        Self { usd, tokens_in, tokens_out, ..Self::default() }
    }

    /// Element-wise accumulate `other` into `self`. USD amounts sum
    /// iff both sides carry a value.
    pub fn accumulate(&mut self, other: &Cost) {
        self.tokens_in = self.tokens_in.saturating_add(other.tokens_in);
        self.tokens_out = self.tokens_out.saturating_add(other.tokens_out);
        self.usd = match (self.usd, other.usd) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
    }
}

/// Sensitive operator identity. The `#[derive(Sensitive)]` in
/// `knotch-derive` wraps this with tracing-redacted Debug/Serialize.
/// Until the derive lands we implement [`Sensitive`] manually.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Person(pub CompactString);

impl Sensitive for Person {}

/// Sensitive agent instance identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub CompactString);

impl Sensitive for AgentId {}

/// Non-sensitive model identifier (e.g. `claude-opus-4-7`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(pub CompactString);

/// Non-sensitive harness identifier (e.g. `claude-code/1.0`,
/// `cursor/0.45`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Harness(pub CompactString);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_accumulates_tokens_and_usd() {
        let mut a = Cost { usd: Some(Decimal::new(1_00, 2)), tokens_in: 5, tokens_out: 7 };
        let b = Cost { usd: Some(Decimal::new(2_50, 2)), tokens_in: 10, tokens_out: 1 };
        a.accumulate(&b);
        assert_eq!(a.tokens_in, 15);
        assert_eq!(a.tokens_out, 8);
        assert_eq!(a.usd, Some(Decimal::new(3_50, 2)));
    }

    #[test]
    fn cost_accumulates_usd_when_one_side_is_none() {
        let mut a = Cost::default();
        let b = Cost { usd: Some(Decimal::new(4_00, 2)), tokens_in: 0, tokens_out: 0 };
        a.accumulate(&b);
        assert_eq!(a.usd, Some(Decimal::new(4_00, 2)));
    }
}
