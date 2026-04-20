//! Built-in pure projections.
//!
//! User-defined projections implement the `Projection<W>` trait. The
//! built-ins here cover the universal views that every workflow needs
//! (current phase, shipped milestones, total cost, supersede-aware
//! effective-events view).

use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::{
    causation::{AgentId, ModelId, Principal},
    event::{Event, EventBody, ToolCallFailureReason},
    id::EventId,
    log::Log,
    status::StatusId,
    time::Timestamp,
    workflow::{MilestoneKind as _, WorkflowKind},
};

/// Current phase: the first required phase that has neither completed
/// nor been skipped. Returns `None` if all required phases have been
/// resolved.
///
/// Forward-looking — answers "what's next?". For the backward-
/// looking view use [`last_completed_phase`].
pub fn current_phase<W: WorkflowKind>(workflow: &W, log: &Log<W>) -> Option<W::Phase> {
    let effective = effective_events(log);
    let resolved: FxHashSet<_> = effective
        .iter()
        .filter_map(|evt| match &evt.body {
            EventBody::PhaseCompleted { phase, .. } | EventBody::PhaseSkipped { phase, .. } => {
                Some(phase.clone())
            }
            _ => None,
        })
        .collect();

    let scope = effective.iter().find_map(|evt| match &evt.body {
        EventBody::UnitCreated { scope } => Some(scope.clone()),
        _ => None,
    })?;

    workflow.required_phases(&scope).iter().find(|p| !resolved.contains(*p)).cloned()
}

/// Most recently completed phase on the effective log, or `None` if
/// no `PhaseCompleted` has been recorded (or every such event has
/// been superseded).
///
/// Backward-looking counterpart to [`current_phase`] — answers
/// "what was done most recently?". `PhaseSkipped` does NOT count as
/// "completed" here; only real `PhaseCompleted` events qualify.
///
/// Use cases: progress bars (last-resolved vs next-pending), resume
/// semantics ("pick up where we left off"), audit reports.
pub fn last_completed_phase<W: WorkflowKind>(log: &Log<W>) -> Option<W::Phase> {
    effective_events(log).into_iter().rev().find_map(|evt| match evt.body {
        EventBody::PhaseCompleted { phase, .. } => Some(phase),
        _ => None,
    })
}

/// Effective events — replay order minus every event that any later
/// `EventSuperseded` points at.
///
/// ## Semantics
///
/// The relation is **single-shot**, not transitive. For a chain
/// `A supersedes B` followed by `C supersedes A`:
///
/// - `B` is removed (A's supersede stands).
/// - `A` is removed (C's supersede stands).
/// - `C` remains.
///
/// `B` does **not** resurrect when its superseder `A` is itself
/// superseded — supersede is a "mark as dead" operation, not a
/// "replace with" operation. If resurrection is the intent, emit a
/// fresh proposal instead of chaining supersedes.
///
/// ## Performance
///
/// `O(n)` in log length — a single pass builds the `superseded` set,
/// a second pass filters. No recursion, no nested lookups.
#[must_use]
pub fn effective_events<W: WorkflowKind>(log: &Log<W>) -> Vec<Event<W>> {
    let superseded: FxHashSet<EventId> = log
        .events()
        .iter()
        .filter_map(|evt| match &evt.body {
            EventBody::EventSuperseded { target, .. } => Some(*target),
            _ => None,
        })
        .collect();

    log.events().iter().filter(|evt| !superseded.contains(&evt.id)).cloned().collect()
}

/// Latest authoritative status, or `None` when the log carries no
/// `StatusTransitioned` events.
#[must_use]
pub fn current_status<W: WorkflowKind>(log: &Log<W>) -> Option<StatusId> {
    effective_events(log).iter().rev().find_map(|evt| match &evt.body {
        EventBody::StatusTransitioned { target, .. } => Some(target.clone()),
        _ => None,
    })
}

/// One `SubagentCompleted` entry surfaced by [`subagents`].
///
/// Narrow summary structure rather than a raw `&EventBody` so the
/// projection API stays stable when the body adds fields — callers
/// pattern-match on this struct, not on `EventBody::SubagentCompleted`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubagentEntry {
    /// Harness-assigned subagent id.
    pub agent_id: AgentId,
    /// Subagent type tag (`"Explore"`, `"Plan"`, adopter-custom), if
    /// the harness classified it.
    pub agent_type: Option<CompactString>,
    /// Absolute path to the subagent's transcript JSONL, if any.
    pub transcript_path: Option<CompactString>,
    /// Last-visible assistant message (capped, see
    /// `crates/knotch-cli/src/cmd/hook/record_subagent.rs`), if any.
    pub last_message: Option<CompactString>,
    /// The stamp of the event that recorded the completion — useful
    /// for "subagent roster at time T" queries via `load_until`.
    pub at: crate::time::Timestamp,
}

/// Subagents that have completed on this unit, in append order.
/// Supersede-aware via `effective_events` — a superseded
/// `SubagentCompleted` drops out of the roster.
#[must_use]
pub fn subagents<W: WorkflowKind>(log: &Log<W>) -> Vec<SubagentEntry> {
    effective_events(log)
        .iter()
        .filter_map(|evt| match &evt.body {
            EventBody::SubagentCompleted {
                agent_id,
                agent_type,
                transcript_path,
                last_message,
            } => Some(SubagentEntry {
                agent_id: agent_id.clone(),
                agent_type: agent_type.clone(),
                transcript_path: transcript_path.clone(),
                last_message: last_message.clone(),
                at: evt.at,
            }),
            _ => None,
        })
        .collect()
}

/// One `(timestamp, model)` pair on the per-unit model timeline.
///
/// Produced by [`model_timeline`]: the model active at the unit's
/// first event (inferred from `Principal::Agent.model`) plus one
/// entry per `ModelSwitched` event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelTimelineEntry {
    /// Instant at which the model became the active one.
    pub at: Timestamp,
    /// The model active from `at` onward.
    pub model: ModelId,
}

/// Chronological model timeline for the unit.
///
/// Seeded from the **first event that reveals a model** — either
/// an `EventBody::ModelSwitched` (whose `to` field is the model
/// active from that point on) or an event whose
/// `Principal::Agent { model, .. }` carries one. Every subsequent
/// `ModelSwitched` appends in log order.
///
/// Order is chronological by construction: the projection walks
/// `effective_events` in log order (= monotonic timestamp order)
/// and never re-orders. An earlier two-pass seed-then-append
/// implementation could emit a `ModelSwitched` before its "seed"
/// entry whenever the first `Principal::Agent` event came later
/// than some `ModelSwitched` event — this single-pass form makes
/// that reordering structurally impossible.
///
/// Supersede-aware via `effective_events`: a superseded
/// `ModelSwitched` drops out of the timeline.
///
/// Empty when no effective event exposes a model.
///
/// Complexity: `O(n)` in log length.
#[must_use]
pub fn model_timeline<W: WorkflowKind>(log: &Log<W>) -> Vec<ModelTimelineEntry> {
    let mut timeline = Vec::new();
    let mut seeded = false;
    for evt in effective_events(log) {
        if let EventBody::ModelSwitched { to, .. } = &evt.body {
            timeline.push(ModelTimelineEntry { at: evt.at, model: to.clone() });
            seeded = true;
        } else if !seeded && let Principal::Agent { model, .. } = &evt.causation.principal {
            timeline.push(ModelTimelineEntry { at: evt.at, model: model.clone() });
            seeded = true;
        }
    }
    timeline
}

/// One `ToolCallFailed` entry on the per-(tool, call_id) retry
/// timeline surfaced by [`tool_call_timeline`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolCallFailureEntry {
    /// Retry attempt counter (1-indexed, monotonic).
    pub attempt: u32,
    /// Classification carried on the failed event.
    pub reason: ToolCallFailureReason,
    /// Instant the failure was recorded.
    pub at: Timestamp,
}

/// Retry timeline for a specific `(tool, call_id)` pair — one entry
/// per effective `ToolCallFailed` event, sorted by attempt
/// ascending. Empty when the pair has no recorded failures.
///
/// The append-time precondition enforces strict monotonicity of
/// `attempt` per `(tool, call_id)` pair, so in a well-formed log
/// every entry has a distinct attempt value and the sort is
/// trivially total. When the sort encounters equal attempts
/// (possible only in hand-crafted logs that bypass the
/// precondition check), `slice::sort_by_key` is stable and
/// preserves the original log order between ties.
///
/// Complexity: `O(n log n)` — filter is `O(n)`, sort is
/// `O(k log k)` where `k ≤ n` is the count of matching failures.
#[must_use]
pub fn tool_call_timeline<W: WorkflowKind>(
    log: &Log<W>,
    tool: &str,
    call_id: &str,
) -> Vec<ToolCallFailureEntry> {
    let mut entries: Vec<ToolCallFailureEntry> = effective_events(log)
        .into_iter()
        .filter_map(|evt| match evt.body {
            EventBody::ToolCallFailed { tool: ref t, call_id: ref c, attempt, reason }
                if t.as_str() == tool && c.as_str() == call_id =>
            {
                Some(ToolCallFailureEntry { attempt: attempt.get(), reason, at: evt.at })
            }
            _ => None,
        })
        .collect();
    entries.sort_by_key(|e| e.attempt);
    entries
}

/// Milestones that have shipped and not been reverted, in first-ship
/// order. Supersede-aware via `effective_events`.
#[must_use]
pub fn shipped_milestones<W: WorkflowKind>(log: &Log<W>) -> Vec<W::Milestone> {
    let mut shipped: Vec<W::Milestone> = Vec::new();
    for evt in effective_events(log) {
        match &evt.body {
            EventBody::MilestoneShipped { milestone, .. } => {
                if !shipped.iter().any(|m| m.id() == milestone.id()) {
                    shipped.push(milestone.clone());
                }
            }
            EventBody::MilestoneReverted { milestone, .. } => {
                shipped.retain(|m| m.id() != milestone.id());
            }
            _ => {}
        }
    }
    shipped
}
