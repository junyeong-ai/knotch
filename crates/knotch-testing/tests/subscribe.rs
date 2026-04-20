//! Subscribe-stream semantics on InMemoryRepository.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use futures::StreamExt as _;
use knotch_kernel::{
    AppendMode, Causation, PhaseKind, Proposal, Repository, Scope, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{EventBody, SkipKind, SubscribeEvent, SubscribeMode},
};
use knotch_testing::InMemoryRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum P {
    Only,
}
impl PhaseKind for P {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("only")
    }
    fn is_skippable(&self, _: &SkipKind) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum M {}
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum G {}
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct W;
const PHASES: [P; 1] = [P::Only];
impl WorkflowKind for W {
    type Phase = P;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("subscribe-test")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
}

fn seed() -> Proposal<W> {
    Proposal {
        causation: Causation::new(
            Source::Cli,
            Principal::System { service: "test".into() },
            Trigger::Manual,
        ),
        extension: (),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    }
}

fn phase_done() -> Proposal<W> {
    Proposal {
        causation: Causation::new(
            Source::Cli,
            Principal::System { service: "test".into() },
            Trigger::Manual,
        ),
        extension: (),
        body: EventBody::PhaseCompleted {
            phase: P::Only,
            artifacts: knotch_kernel::event::ArtifactList::default(),
        },
        supersedes: None,
    }
}

#[tokio::test]
async fn live_only_misses_events_before_subscribe() {
    let repo = Arc::new(InMemoryRepository::<W>::new(W));
    let unit = UnitId::try_new("s1").unwrap();

    repo.append(&unit, vec![seed()], AppendMode::BestEffort).await.expect("seed");

    let mut stream = repo.subscribe(&unit, SubscribeMode::LiveOnly).await.expect("subscribe");

    repo.append(&unit, vec![phase_done()], AppendMode::BestEffort).await.expect("append");

    let first = stream.next().await.expect("one");
    match first {
        SubscribeEvent::Event(e) => {
            assert!(matches!(e.body, EventBody::PhaseCompleted { .. }));
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn from_beginning_replays_history_then_goes_live() {
    let repo = Arc::new(InMemoryRepository::<W>::new(W));
    let unit = UnitId::try_new("s2").unwrap();

    repo.append(&unit, vec![seed()], AppendMode::BestEffort).await.expect("seed");

    let mut stream = repo.subscribe(&unit, SubscribeMode::FromBeginning).await.expect("subscribe");

    // Historical frame first.
    let historical = stream.next().await.expect("historical");
    match historical {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::UnitCreated { .. })),
        other => panic!("{other:?}"),
    }

    // Then live.
    repo.append(&unit, vec![phase_done()], AppendMode::BestEffort).await.expect("append");
    let live = stream.next().await.expect("live");
    match live {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::PhaseCompleted { .. })),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn from_event_id_skips_up_to_and_including_anchor() {
    let repo = Arc::new(InMemoryRepository::<W>::new(W));
    let unit = UnitId::try_new("s3").unwrap();

    let r = repo
        .append(&unit, vec![seed(), phase_done()], AppendMode::BestEffort)
        .await
        .expect("append");
    let first_id = r.accepted[0].id;

    let mut stream =
        repo.subscribe(&unit, SubscribeMode::FromEventId(first_id)).await.expect("subscribe");

    // Only phase_done should replay.
    let evt = stream.next().await.expect("replayed");
    match evt {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::PhaseCompleted { .. })),
        other => panic!("{other:?}"),
    }
}
