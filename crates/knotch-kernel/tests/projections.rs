//! Built-in projection semantics — model-timeline attribution,
//! ordering, and supersede-awareness.

#![allow(missing_docs)]

use std::borrow::Cow;

use jiff::Timestamp;
use knotch_kernel::{
    Causation, Log, PhaseKind, Scope, UnitId, WorkflowKind,
    causation::{ModelId, Source, Trigger},
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
    Causation::new(Source::Cli, Trigger::Command { name: "test".into() })
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

fn model_switched(from: &str, to: &str) -> EventBody<Wf> {
    EventBody::ModelSwitched { from: ModelId(from.into()), to: ModelId(to.into()) }
}

// --- model_timeline --------------------------------------------------

#[test]
fn model_timeline_records_one_entry_per_model_switched_event() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, plain_causation(), model_switched("sonnet-4-6", "opus-4-7")),
        event(3_000, plain_causation(), model_switched("opus-4-7", "haiku-4-5")),
    ];
    let log = log_from(events);
    let tl = model_timeline(&log);

    assert_eq!(tl.len(), 2);
    assert_eq!(tl[0].model, ModelId("opus-4-7".into()));
    assert_eq!(tl[0].at.as_millisecond(), 2_000);
    assert_eq!(tl[1].model, ModelId("haiku-4-5".into()));
    assert_eq!(tl[1].at.as_millisecond(), 3_000);
}

#[test]
fn model_timeline_preserves_chronological_order() {
    let events = vec![
        event(1_000, plain_causation(), created()),
        event(2_000, plain_causation(), model_switched("sonnet-4-6", "opus-4-7")),
        event(3_000, plain_causation(), model_switched("opus-4-7", "sonnet-4-6")),
    ];
    let log = log_from(events);
    let tl = model_timeline(&log);

    assert!(tl.windows(2).all(|w| w[0].at <= w[1].at), "timeline must be chronological: {tl:?}");
    assert_eq!(tl.len(), 2);
}

#[test]
fn model_timeline_is_empty_when_no_event_records_a_switch() {
    let events = vec![event(1_000, plain_causation(), created())];
    let log = log_from(events);
    assert!(model_timeline(&log).is_empty());
}
