//! FileRepository subscribe-stream semantics.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use futures::StreamExt as _;
use knotch_derive::MilestoneKind;
use knotch_kernel::{
    AppendMode, Causation, CommitStatus, PhaseKind, Proposal, Repository, Scope, UnitId,
    WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{CommitKind, CommitRef, EventBody, SkipKind, SubscribeEvent, SubscribeMode},
};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum Phase { Only }
impl PhaseKind for Phase {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed("only") }
    fn is_skippable(&self, _: &SkipKind) -> bool { false }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
enum Milestone { Alpha }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Gate {}
impl knotch_kernel::GateKind for Gate {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed("") }
}

#[derive(Debug, Clone, Copy)]
struct Wf;
const PHASES: [Phase; 1] = [Phase::Only];
impl WorkflowKind for Wf {
    type Phase = Phase;
    type Milestone = Milestone;
    type Gate = Gate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("filerepo-subscribe-test") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

fn cause() -> Causation {
    Causation::new(Source::Cli, Principal::System { service: "t".into() }, Trigger::Manual)
}

fn p(body: EventBody<Wf>) -> Proposal<Wf> {
    Proposal { causation: cause(), extension: (), body, supersedes: None }
}

#[tokio::test]
async fn live_only_delivers_post_subscribe_events() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("sub-1");
    repo.append(&unit, vec![p(EventBody::UnitCreated { scope: Scope::Standard })],
                AppendMode::BestEffort).await.expect("seed");

    let mut stream = repo.subscribe(&unit, SubscribeMode::LiveOnly).await.expect("sub");
    let repo_cloned = repo.clone();
    let unit_cloned = unit.clone();
    tokio::spawn(async move {
        repo_cloned.append(
            &unit_cloned,
            vec![p(EventBody::MilestoneShipped {
                milestone: Milestone::Alpha,
                commit: CommitRef::new("abc"),
                commit_kind: CommitKind::Feat,
                status: CommitStatus::Verified,
            })],
            AppendMode::BestEffort,
        ).await.expect("live append");
    });
    let evt = stream.next().await.expect("live");
    match evt {
        SubscribeEvent::Event(e) => {
            assert!(matches!(e.body, EventBody::MilestoneShipped { .. }));
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn from_beginning_replays_history_then_live() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("sub-2");
    repo.append(&unit, vec![p(EventBody::UnitCreated { scope: Scope::Standard })],
                AppendMode::BestEffort).await.expect("seed");

    let mut stream = repo.subscribe(&unit, SubscribeMode::FromBeginning).await.expect("sub");
    let first = stream.next().await.expect("historical");
    match first {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::UnitCreated { .. })),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn from_event_id_replays_after_anchor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("sub-3");
    let report = repo.append(
        &unit,
        vec![
            p(EventBody::UnitCreated { scope: Scope::Standard }),
            p(EventBody::MilestoneShipped {
                milestone: Milestone::Alpha,
                commit: CommitRef::new("abc"),
                commit_kind: CommitKind::Feat,
                status: CommitStatus::Verified,
            }),
        ],
        AppendMode::BestEffort,
    ).await.expect("seed");
    let first_id = report.accepted[0].id;

    let mut stream = repo
        .subscribe(&unit, SubscribeMode::FromEventId(first_id))
        .await
        .expect("sub");
    let evt = stream.next().await.expect("replay");
    match evt {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::MilestoneShipped { .. })),
        other => panic!("{other:?}"),
    }
}
