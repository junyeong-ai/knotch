//! Built-in projection semantics — attribution, ordering, and
//! supersede-awareness for the cost + timeline projections.

#![allow(missing_docs)]

use std::borrow::Cow;

use jiff::Timestamp;
use knotch_kernel::{
    Causation, CommitStatus, Log, PhaseKind, Scope, UnitId, WorkflowKind,
    causation::{AgentId, Cost, Harness, ModelId, Principal, Source, Trigger},
    event::{ArtifactList, CommitKind, CommitRef, Event, EventBody},
    id::EventId,
    project::{cost_by_milestone, cost_by_phase, model_timeline, total_cost},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// --- Workflow fixture -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum P {
    One,
    Two,
}
impl PhaseKind for P {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            P::One => "one",
            P::Two => "two",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct M(String);
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct G(String);
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Wf;
const PHASES: [P; 2] = [P::One, P::Two];
impl WorkflowKind for Wf {
    type Phase = P;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("projection-fixture")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> Cow<'_, [Self::Phase]> {
        Cow::Borrowed(&PHASES)
    }
}

// --- Helpers ----------------------------------------------------------

fn plain_causation() -> Causation {
    Causation::new(
        Source::Cli,
        Principal::System { service: "t".into() },
        Trigger::Command { name: "test".into() },
    )
}

fn agent_causation(model: &str) -> Causation {
    Causation::new(
        Source::Hook,
        Principal::Agent {
            agent_id: AgentId("agent-a".into()),
            model: ModelId(model.into()),
            harness: Harness("claude-code".into()),
        },
        Trigger::Command { name: "test".into() },
    )
}

fn causation_with_cost(tokens_in: u32, tokens_out: u32, usd: Option<Decimal>) -> Causation {
    plain_causation().with_cost(Cost::new(usd, tokens_in, tokens_out))
}

fn event(at_ms: i64, causation: Causation, body: EventBody<Wf>) -> Event<Wf> {
    Event {
        id: EventId::new_v7(),
        at: Timestamp::from_millisecond(at_ms).unwrap(),
        causation,
        extension: (),
        body,
        supersedes: None,
    }
}

fn log_from(events: Vec<Event<Wf>>) -> Log<Wf> {
    Log::from_events(UnitId::try_new("demo").unwrap(), events)
}

fn created() -> EventBody<Wf> {
    EventBody::UnitCreated { scope: Scope::Standard }
}

/// Body used to stand in for "work that carries cost but neither
/// resolves a phase nor ships a milestone". `Log::from_events`
/// skips precondition dispatch, so sprinkling extra `UnitCreated`
/// envelopes is legal at this layer and keeps the fixture minimal.
fn work_body() -> EventBody<Wf> {
    created()
}

fn milestone(id: &str) -> EventBody<Wf> {
    EventBody::MilestoneShipped {
        commit: CommitRef::new("a".repeat(40)),
        commit_kind: CommitKind::Feat,
        milestone: M(id.to_owned()),
        status: CommitStatus::Verified,
    }
}

fn phase_completed(phase: P) -> EventBody<Wf> {
    EventBody::PhaseCompleted { phase, artifacts: ArtifactList::default() }
}

// --- cost_by_phase ---------------------------------------------------

#[test]
fn cost_by_phase_attributes_events_to_first_unresolved_required_phase() {
    // UnitCreated → 2 work events (P::One active) → PhaseCompleted(One)
    // → 1 work event (P::Two active) → PhaseCompleted(Two). The
    // completion events are themselves billed to the phase they
    // complete ("work that closes the phase").
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, causation_with_cost(10, 5, Some(Decimal::new(1, 2))), work_body()),
        event(3_000, causation_with_cost(20, 7, Some(Decimal::new(2, 2))), work_body()),
        event(4_000, causation_with_cost(1, 1, None), phase_completed(P::One)),
        event(5_000, causation_with_cost(30, 11, Some(Decimal::new(5, 2))), work_body()),
        event(6_000, causation_with_cost(2, 2, None), phase_completed(P::Two)),
    ];
    let log = log_from(events);
    let buckets = cost_by_phase(&Wf, &log);

    let one = buckets.get(&P::One).expect("phase One has cost");
    assert_eq!(one.tokens_in, 31);
    assert_eq!(one.tokens_out, 13);
    assert_eq!(one.usd, Some(Decimal::new(3, 2)));

    let two = buckets.get(&P::Two).expect("phase Two has cost");
    assert_eq!(two.tokens_in, 32);
    assert_eq!(two.tokens_out, 13);
    assert_eq!(two.usd, Some(Decimal::new(5, 2)));
}

#[test]
fn cost_by_phase_drops_events_after_all_required_phases_resolved() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, causation_with_cost(5, 5, None), phase_completed(P::One)),
        event(3_000, causation_with_cost(5, 5, None), phase_completed(P::Two)),
        event(4_000, causation_with_cost(100, 100, None), milestone("m-post-resolve")),
    ];
    let log = log_from(events);
    let buckets = cost_by_phase(&Wf, &log);

    assert_eq!(buckets.values().map(|c| c.tokens_in).sum::<u32>(), 10);
    let total = total_cost(&log);
    assert_eq!(total.tokens_in, 110, "total_cost still reflects every event");
}

#[test]
fn cost_by_phase_empty_when_unit_not_created() {
    let log: Log<Wf> = log_from(vec![]);
    assert!(cost_by_phase(&Wf, &log).is_empty());
}

// --- cost_by_milestone -----------------------------------------------

#[test]
fn cost_by_milestone_buckets_pending_cost_into_next_shipped_milestone() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, causation_with_cost(10, 10, None), work_body()),
        event(3_000, causation_with_cost(10, 10, None), milestone("m1")),
        event(4_000, causation_with_cost(20, 20, None), work_body()),
        event(5_000, causation_with_cost(5, 5, None), milestone("m2")),
    ];
    let log = log_from(events);
    let entries = cost_by_milestone(&log);

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].milestone, M("m1".into()));
    assert_eq!(entries[0].cost.tokens_in, 20);
    assert_eq!(entries[1].milestone, M("m2".into()));
    assert_eq!(entries[1].cost.tokens_in, 25);
}

#[test]
fn cost_by_milestone_leaves_trailing_cost_unbucketed() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, causation_with_cost(10, 10, None), milestone("m1")),
        event(3_000, causation_with_cost(30, 30, None), work_body()),
    ];
    let log = log_from(events);
    let entries = cost_by_milestone(&log);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].cost.tokens_in, 10);
    let total = total_cost(&log);
    let bucketed: u32 = entries.iter().map(|e| e.cost.tokens_in).sum();
    assert_eq!(total.tokens_in - bucketed, 30, "30 tokens trail past the last milestone");
}

// --- model_timeline --------------------------------------------------

#[test]
fn model_timeline_seeds_from_first_agent_principal_then_appends_switches() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, agent_causation("sonnet-4-6"), work_body()),
        event(
            3_000,
            agent_causation("sonnet-4-6"),
            EventBody::ModelSwitched {
                from: ModelId("sonnet-4-6".into()),
                to: ModelId("opus-4-7".into()),
            },
        ),
    ];
    let log = log_from(events);
    let tl = model_timeline(&log);

    assert_eq!(tl.len(), 2);
    assert_eq!(tl[0].model, ModelId("sonnet-4-6".into()));
    assert_eq!(tl[0].at.as_millisecond(), 2_000);
    assert_eq!(tl[1].model, ModelId("opus-4-7".into()));
    assert_eq!(tl[1].at.as_millisecond(), 3_000);
}

#[test]
fn model_timeline_preserves_chronological_order_when_switch_precedes_agent_principal() {
    // Regression for a two-pass bug: if the first Agent-principal
    // event came *after* a `ModelSwitched` event, the old
    // seed-then-append algorithm emitted
    //     [(t_agent, model_agent), (t_switch, model_switch)]
    // which is out of chronological order when t_switch < t_agent.
    // The single-pass rewrite seeds from whichever comes first —
    // here, the ModelSwitched — so the timeline is monotonic by
    // construction.
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(
            2_000,
            plain_causation(),
            EventBody::ModelSwitched {
                from: ModelId("sonnet-4-6".into()),
                to: ModelId("opus-4-7".into()),
            },
        ),
        event(3_000, agent_causation("opus-4-7"), work_body()),
    ];
    let log = log_from(events);
    let tl = model_timeline(&log);

    assert!(tl.windows(2).all(|w| w[0].at <= w[1].at), "timeline must be chronological: {tl:?}");
    assert_eq!(tl.len(), 1, "post-seed Agent events with matching model add nothing");
    assert_eq!(tl[0].at.as_millisecond(), 2_000);
    assert_eq!(tl[0].model, ModelId("opus-4-7".into()));
}

#[test]
fn model_timeline_is_empty_when_no_event_exposes_a_model() {
    let events = vec![event(1_000, plain_causation(), created())];
    let log = log_from(events);
    assert!(model_timeline(&log).is_empty());
}
