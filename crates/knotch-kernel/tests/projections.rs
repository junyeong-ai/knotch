//! Built-in projection semantics — attribution, ordering, and
//! supersede-awareness for the model-timeline projection.

#![allow(missing_docs)]

use std::borrow::Cow;

use jiff::Timestamp;
use knotch_kernel::{
    Causation, Log, PhaseKind, Scope, UnitId, WorkflowKind,
    causation::{AgentId, ModelId, Principal, Source, Trigger},
    event::{Event, EventBody},
    id::EventId,
    project::model_timeline,
};
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
        Principal::Agent { agent_id: AgentId("agent-a".into()), model: ModelId(model.into()) },
        Trigger::Command { name: "test".into() },
    )
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

/// Body used to stand in for "work that neither resolves a phase nor
/// ships a milestone". `Log::from_events` skips precondition dispatch,
/// so sprinkling extra `UnitCreated` envelopes is legal at this layer
/// and keeps the fixture minimal.
fn work_body() -> EventBody<Wf> {
    created()
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
