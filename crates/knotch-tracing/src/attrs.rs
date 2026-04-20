//! Canonical attribute keys emitted on knotch tracing spans.
//!
//! Keys are grouped by subsystem (`repo`, `observer`, `unit`,
//! `event`, `principal`, `agent`, `session`, `trace`, `cost`,
//! `status`). Changing any constant is a breaking change — guarded
//! by `cargo-public-api` and `cargo-semver-checks`.

/// Attribute-key constants. Accessing them by reference is idiomatic;
/// the type is zero-sized.
pub struct Attrs;

impl Attrs {
    // --- unit ---
    /// `knotch.unit.id` — the `UnitId` string.
    pub const UNIT_ID: &'static str = "knotch.unit.id";

    // --- event ---
    /// `knotch.event.id` — ULID/UUIDv7 of the event.
    pub const EVENT_ID: &'static str = "knotch.event.id";
    /// `knotch.event.kind` — body-variant tag (e.g. `milestone_shipped`).
    pub const EVENT_KIND: &'static str = "knotch.event.kind";

    // --- repository ---
    /// `knotch.repository.op` — one of `append`, `load`, `subscribe`,
    /// `list_units`, `with_cache`.
    pub const REPOSITORY_OP: &'static str = "knotch.repository.op";
    /// `knotch.repository.outcome` — `accepted` / `rejected`.
    pub const REPOSITORY_OUTCOME: &'static str = "knotch.repository.outcome";

    // --- observer ---
    /// `knotch.observer.name` — observer's `Observer::name()`.
    pub const OBSERVER_NAME: &'static str = "knotch.observer.name";

    // --- reconcile ---
    /// `knotch.reconcile.accepted` — number of accepted proposals.
    pub const RECONCILE_ACCEPTED: &'static str = "knotch.reconcile.accepted";
    /// `knotch.reconcile.rejected` — number of rejected proposals.
    pub const RECONCILE_REJECTED: &'static str = "knotch.reconcile.rejected";

    // --- principal ---
    /// `knotch.principal.kind` — `human` / `agent` / `system`.
    pub const PRINCIPAL_KIND: &'static str = "knotch.principal.kind";

    // --- agent (when principal is agent) ---
    /// `knotch.agent.id.hash` — BLAKE3 of `AgentId` (sensitive).
    pub const AGENT_ID_HASH: &'static str = "knotch.agent.id.hash";
    /// `knotch.agent.model` — `ModelId` (public).
    pub const AGENT_MODEL: &'static str = "knotch.agent.model";
    /// `knotch.agent.harness` — `Harness` (public).
    pub const AGENT_HARNESS: &'static str = "knotch.agent.harness";

    // --- session / trace ---
    /// `knotch.session.id` — conversation / run scope.
    pub const SESSION_ID: &'static str = "knotch.session.id";
    /// `knotch.trace.id` — OTel-compatible 128-bit trace id.
    pub const TRACE_ID: &'static str = "knotch.trace.id";

    // --- cost ---
    /// `knotch.cost.usd` — decimal USD cost.
    pub const COST_USD: &'static str = "knotch.cost.usd";
    /// `knotch.cost.tokens_in` — prompt tokens.
    pub const COST_TOKENS_IN: &'static str = "knotch.cost.tokens_in";
    /// `knotch.cost.tokens_out` — completion tokens.
    pub const COST_TOKENS_OUT: &'static str = "knotch.cost.tokens_out";

    // --- status ---
    /// `knotch.status.forced` — `true` when a forced transition.
    pub const STATUS_FORCED: &'static str = "knotch.status.forced";
}
