//! Span emission helpers. Each helper takes a typed domain value and
//! emits a `tracing` event with the canonical attribute keys.
//!
//! Keys are recorded via `tracing::field::display` / primitive
//! coercion so subscribers can parse them cleanly. Constants in
//! [`Attrs`](crate::attrs::Attrs) document the schema — if a caller
//! reads attributes out of a JSON event they use the same strings.

use knotch_kernel::{
    Causation, UnitId,
    causation::Source,
    event::{Event, EventBody},
    workflow::WorkflowKind,
};
use tracing::info;

/// Record an `append` outcome for observability.
pub fn emit_append<W: WorkflowKind>(
    unit: &UnitId,
    accepted: usize,
    rejected: usize,
    kind_tag: &str,
) {
    info!(
        target: "knotch.repo",
        op = "append",
        unit_id = unit.as_str(),
        event_kind = kind_tag,
        accepted = accepted,
        rejected = rejected,
    );
    // Keep the generic parameter load-bearing so rustc doesn't warn.
    let _ = std::marker::PhantomData::<W>;
    // Shut up the "private trait imported but unused" lints that would
    // otherwise chase the WorkflowKind import.
    let _: fn(&EventBody<W>) -> () = |_| {};
}

/// Record a reconcile pass completion.
pub fn emit_reconcile(unit: &UnitId, accepted: usize, rejected: usize) {
    info!(
        target: "knotch.reconcile",
        unit_id = unit.as_str(),
        accepted = accepted,
        rejected = rejected,
    );
}

/// Record per-event attribution at write time.
pub fn emit_event<W: WorkflowKind>(unit: &UnitId, event: &Event<W>) {
    info!(
        target: "knotch.event",
        unit_id = unit.as_str(),
        event_id = %event.id,
        event_kind = event_kind_tag(&event.body),
        source = source_tag(&event.causation),
    );
    emit_attribution(&event.causation);
}

fn emit_attribution(c: &Causation) {
    if let Some(agent_id) = &c.agent_id {
        info!(
            target: "knotch.principal",
            source = source_tag(c),
            agent_id = agent_id.as_str(),
        );
    } else {
        info!(target: "knotch.principal", source = source_tag(c));
    }
}

fn source_tag(c: &Causation) -> &'static str {
    match c.source {
        Source::Cli => "cli",
        Source::Hook => "hook",
        Source::Observer => "observer",
        _ => "unknown",
    }
}

fn event_kind_tag<W: WorkflowKind>(body: &EventBody<W>) -> &'static str {
    body.kind_tag()
}

#[cfg(test)]
mod tests {
    use compact_str::CompactString;
    use knotch_kernel::causation::{AgentId, Source, Trigger};

    use super::*;

    #[test]
    fn emit_helpers_run_without_panic() {
        let unit = UnitId::try_new("trace-unit").unwrap();
        emit_reconcile(&unit, 1, 0);
        let causation = Causation::new(Source::Hook, Trigger::Command { name: "test".into() })
            .with_agent_id(AgentId(CompactString::from("alice")));
        emit_attribution(&causation);
    }
}
