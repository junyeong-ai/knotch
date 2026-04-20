//! Span emission helpers. Each helper takes a typed domain value and
//! emits a `tracing` event with the canonical attribute keys.
//!
//! Keys are recorded via `tracing::field::display` / primitive
//! coercion so subscribers can parse them cleanly. Constants in
//! [`Attrs`](crate::attrs::Attrs) document the schema — if a caller
//! reads attributes out of a JSON event they use the same strings.

use knotch_kernel::{
    Causation, UnitId,
    causation::{Cost, Principal},
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
        principal_kind = principal_kind(&event.causation),
    );
    if let Some(cost) = &event.causation.cost {
        emit_cost(cost);
    }
    emit_principal(&event.causation);
}

fn emit_cost(cost: &Cost) {
    let usd = cost.usd.map(|d| d.to_string()).unwrap_or_default();
    info!(
        target: "knotch.cost",
        usd = %usd,
        tokens_in = cost.tokens_in,
        tokens_out = cost.tokens_out,
    );
}

fn emit_principal(c: &Causation) {
    match &c.principal {
        Principal::Agent { agent_id, model, harness } => {
            let hashed = blake3::hash(agent_id.0.as_bytes()).to_hex().to_string();
            info!(
                target: "knotch.principal",
                principal_kind = "agent",
                agent_id_hash = &hashed[..16],
                agent_model = model.0.as_str(),
                agent_harness = harness.0.as_str(),
            );
        }
        Principal::Human { .. } => {
            info!(target: "knotch.principal", principal_kind = "human");
        }
        Principal::System { service } => {
            info!(
                target: "knotch.principal",
                principal_kind = "system",
                service = service.as_str(),
            );
        }
        _ => {
            info!(target: "knotch.principal", principal_kind = "unknown");
        }
    }
}

fn principal_kind(c: &Causation) -> &'static str {
    match c.principal {
        Principal::Human { .. } => "human",
        Principal::Agent { .. } => "agent",
        Principal::System { .. } => "system",
        _ => "unknown",
    }
}

fn event_kind_tag<W: WorkflowKind>(body: &EventBody<W>) -> &'static str {
    body.kind_tag()
}

#[cfg(test)]
mod tests {
    use compact_str::CompactString;
    use knotch_kernel::causation::{AgentId, Harness, ModelId, Principal, Source, Trigger};

    use super::*;

    #[test]
    fn emit_helpers_run_without_panic() {
        let unit = UnitId::new("trace-unit");
        emit_reconcile(&unit, 1, 0);
        let causation = Causation::new(
            Source::Agent,
            Principal::Agent {
                agent_id: AgentId(CompactString::from("alice")),
                model: ModelId(CompactString::from("claude-opus-4-7")),
                harness: Harness(CompactString::from("claude-code/1.0")),
            },
            Trigger::Manual,
        );
        emit_principal(&causation);
    }
}
