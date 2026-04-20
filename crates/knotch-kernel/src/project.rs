//! Built-in pure projections.
//!
//! User-defined projections implement the `Projection<W>` trait. The
//! built-ins here cover the universal views that every workflow needs
//! (current phase, shipped milestones, total cost, supersede-aware
//! effective-events view).

use rustc_hash::FxHashSet;

use crate::{
    causation::Cost,
    event::{Event, EventBody},
    id::EventId,
    log::Log,
    status::StatusId,
    workflow::{MilestoneKind as _, WorkflowKind},
};

/// Current phase: the first required phase that has neither completed
/// nor been skipped. Returns `None` if all required phases have been
/// resolved.
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

/// Sum of every `Causation::cost` value on effective events.
#[must_use]
pub fn total_cost<W: WorkflowKind>(log: &Log<W>) -> Cost {
    let mut total = Cost::default();
    for evt in effective_events(log) {
        if let Some(cost) = &evt.causation.cost {
            total.accumulate(cost);
        }
    }
    total
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
