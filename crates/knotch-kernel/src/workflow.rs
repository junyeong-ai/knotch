//! `WorkflowKind` and its associated-type traits.
//!
//! Knotch is parameterized by a single type `W: WorkflowKind` carrying
//! four associated types: `Phase`, `Milestone`, `Gate`, `Extension`.
//! This is the "single-bound" design of RFC 0002.
//!
//! The kernel defines traits only; the canonical `WorkflowKind` impl
//! (`knotch_workflow::Knotch`) lives in `knotch-workflow`, and
//! adopter-specific impls live in the adopter's own crate.

use std::{borrow::Cow, fmt::Debug, hash::Hash};

use serde::{Serialize, de::DeserializeOwned};

use crate::{event::SkipKind, scope::Scope};

/// Top-level workflow marker.
///
/// Implementors are typically zero-sized types or thin enums; the
/// information lives in the associated types. `#[workflow]` in
/// `knotch-derive` generates canonical impls with compile-time
/// invariants (phase acyclicity, milestone-id uniqueness).
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a knotch WorkflowKind",
    note = "derive `#[workflow]` on your workflow marker type or implement \
            `WorkflowKind` manually",
)]
pub trait WorkflowKind: Clone + core::fmt::Debug + Send + Sync + 'static {
    /// The set of phases this workflow recognizes.
    type Phase: PhaseKind;
    /// The set of milestones this workflow ships.
    type Milestone: MilestoneKind;
    /// The set of gates this workflow records.
    type Gate: GateKind;
    /// Workflow-specific per-event payload.
    type Extension: ExtensionKind;

    /// Stable workflow name — appears in the log-file header and
    /// namespaces the default `fingerprint_salt`. Typed workflows
    /// return a compile-time constant; runtime-configurable workflows
    /// (e.g. `ConfigWorkflow`) return the configured name.
    fn name(&self) -> Cow<'_, str>;

    /// Wire-format schema version. Bumped for backwards-incompatible
    /// changes to the workflow's shape.
    fn schema_version(&self) -> u32;

    /// Which phases must complete (or be explicitly skipped) before
    /// a unit may transition to a terminal status.
    ///
    /// Scope-based skipping: a phase omitted from the required list
    /// for a given scope is treated by projections as already
    /// resolved. For mid-flight skip with rationale, use
    /// [`PhaseKind::is_skippable`] + `PhaseSkipped`.
    fn required_phases(&self, scope: &Scope) -> Cow<'_, [Self::Phase]>;

    /// Salt mixed into every fingerprint so two workflows cannot
    /// collide on dedup tuples. Defaults to `name()` bytes — two
    /// configured workflows with different names therefore occupy
    /// disjoint fingerprint namespaces automatically.
    fn fingerprint_salt(&self) -> Cow<'_, [u8]> {
        match self.name() {
            Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
            Cow::Owned(s) => Cow::Owned(s.into_bytes()),
        }
    }

    /// Minimum rationale length enforced by preconditions. Workflows
    /// with stricter policies override; defaults to
    /// [`DEFAULT_MIN_RATIONALE_CHARS`](crate::rationale::DEFAULT_MIN_RATIONALE_CHARS).
    fn min_rationale_chars(&self) -> usize {
        crate::rationale::DEFAULT_MIN_RATIONALE_CHARS
    }

    /// Is `status` a terminal lifecycle state for this workflow?
    ///
    /// The Phase × Status cross-invariant requires that a non-forced
    /// transition into a terminal status only succeeds when every
    /// required phase has been completed or skipped. Default returns
    /// `false` for every status — workflows that distinguish terminal
    /// states override.
    fn is_terminal_status(&self, _status: &crate::status::StatusId) -> bool {
        false
    }

    /// Parse a human-supplied phase id into this workflow's `Phase`
    /// variant. Default assumes serde-rendered form (snake_case).
    fn parse_phase(&self, text: &str) -> Option<Self::Phase> {
        let value = serde_json::Value::String(text.to_owned());
        serde_json::from_value(value).ok()
    }

    /// Parse a human-supplied gate id into this workflow's `Gate`
    /// variant. Mirrors [`parse_phase`](Self::parse_phase).
    fn parse_gate(&self, text: &str) -> Option<Self::Gate> {
        let value = serde_json::Value::String(text.to_owned());
        serde_json::from_value(value).ok()
    }

    /// Parse a human-supplied milestone id into this workflow's
    /// `Milestone` variant. Workflows with opaque milestone types
    /// (ticket ids, hash prefixes) override.
    fn parse_milestone(&self, text: &str) -> Option<Self::Milestone> {
        let value = serde_json::Value::String(text.to_owned());
        serde_json::from_value(value).ok()
    }

    /// Prerequisite gates that must already appear on the log before
    /// `gate` can be recorded. Kernel-enforced via
    /// `EventBody::check_precondition` on `GateRecorded`.
    ///
    /// Default delegates to [`GateKind::prerequisites`] — typed
    /// workflows (enum-backed Gate types) declare the graph per
    /// variant. Runtime-configured workflows (`ConfigWorkflow`,
    /// observer-driven workflows) override this method to look the
    /// graph up from their own data.
    ///
    /// The lifetime parameter `'a` lets either source — `&self` or
    /// `&gate` — own the returned slice.
    fn prerequisites_for<'a>(
        &'a self,
        gate: &'a Self::Gate,
    ) -> Cow<'a, [Self::Gate]> {
        GateKind::prerequisites(gate)
    }

    /// Whether `phase` accepts `reason` as a `PhaseSkipped` cause.
    /// Kernel-enforced via `EventBody::check_precondition` on
    /// `PhaseSkipped`. Default delegates to
    /// [`PhaseKind::is_skippable`]; ConfigWorkflow overrides to
    /// consult the `accepts_skips` list declared in config.
    fn accepts_skip_for(
        &self,
        phase: &Self::Phase,
        reason: &crate::event::SkipKind,
    ) -> bool {
        phase.is_skippable(reason)
    }

    /// Canonical status vocabulary for this workflow.
    ///
    /// Returned in snake_case, matching the serialized form of
    /// [`StatusId`](crate::status::StatusId). CLI commands that
    /// accept a status argument (currently `knotch transition`) warn
    /// when the user supplies a string not in this list — guarding
    /// against typos while leaving the universe open (the list is
    /// a *hint*, not an invariant).
    ///
    /// Default: empty slice (no validation, open universe). Presets
    /// with a curated status set override this.
    ///
    /// Returns borrowed `Cow<str>` values so typed workflows can
    /// serve compile-time string literals without allocation while
    /// runtime-configured workflows (`ConfigWorkflow`) can serve
    /// `&self.spec.known_statuses[i]` without leaking.
    fn known_statuses(&self) -> Vec<Cow<'_, str>> {
        Vec::new()
    }
}

/// Trait for a workflow's `Phase` type.
///
/// Phases are identified by their kebab-case id (`id()`). Per-phase
/// metadata that affects kernel dispatch — whether an explicit
/// `PhaseSkipped` event is accepted for a given reason — lives
/// behind [`WorkflowKind::accepts_skip_for`]. The default delegates
/// to [`is_skippable`](Self::is_skippable) here so typed workflows
/// can answer per-variant; runtime-configured workflows override
/// the workflow-level hook to consult their config.
pub trait PhaseKind:
    Clone + Eq + Ord + Hash + Serialize + DeserializeOwned + Debug + Send + Sync + 'static
{
    /// Machine-readable phase id.
    fn id(&self) -> Cow<'_, str>;

    /// Whether an explicit `PhaseSkipped` event against this phase
    /// with `reason` is admissible. Defaults to `false` — typed
    /// workflows opt in per variant when they need mid-flight skips.
    /// ConfigWorkflow ignores this (see
    /// [`WorkflowKind::accepts_skip_for`] for config-level rules).
    fn is_skippable(&self, _reason: &SkipKind) -> bool {
        false
    }
}

/// Trait for a workflow's `Milestone` type.
pub trait MilestoneKind:
    Clone + Eq + Hash + Serialize + DeserializeOwned + Debug + Send + Sync + 'static
{
    /// Globally-unique milestone identifier within a unit.
    fn id(&self) -> Cow<'_, str>;
}

/// Trait for a workflow's `Gate` type.
pub trait GateKind:
    Clone + Eq + Hash + Serialize + DeserializeOwned + Debug + Send + Sync + 'static
{
    /// Machine-readable gate id.
    fn id(&self) -> Cow<'_, str>;

    /// Gates that must already appear on the log before `self` can be
    /// recorded. The default is empty — workflows without a gate
    /// ladder inherit it automatically.
    ///
    /// Enforced structurally by
    /// [`EventBody::check_precondition`](crate::event::EventBody::check_precondition)
    /// for `GateRecorded`: every prerequisite must be present on the
    /// log or the append is rejected with
    /// `PreconditionError::GateOutOfOrder`. There is no caller-side
    /// escape hatch — the invariant is kernel-enforced.
    fn prerequisites(&self) -> Cow<'_, [Self]>
    where
        Self: Sized,
    {
        Cow::Borrowed(&[])
    }
}

/// Trait for a workflow's per-event `Extension` payload.
///
/// `()` is the canonical "no extension" type and is the default for
/// workflows that don't need extras.
pub trait ExtensionKind:
    Clone + Serialize + DeserializeOwned + Debug + Send + Sync + 'static
{
    /// Extension-contributed append-time precondition hook.
    ///
    /// Evaluated *after* the body-level precondition has succeeded
    /// (so extension logic can assume the body invariants hold).
    /// The default impl accepts everything — workflows with typed
    /// extensions override only when they want to enforce an invariant
    /// tied to their payload (e.g. "cost must be attributed to an
    /// allow-listed model").
    ///
    /// # Errors
    /// Return `PreconditionError::Extension(…)` with a descriptive
    /// message to reject the append.
    fn check_extension<W>(
        &self,
        _ctx: &crate::precondition::AppendContext<'_, W>,
    ) -> Result<(), crate::error::PreconditionError>
    where
        W: WorkflowKind<Extension = Self>,
    {
        Ok(())
    }
}

impl ExtensionKind for () {}
