//! FileRepository subscribe-stream semantics.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use compact_str::CompactString;
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
enum Phase {
    Only,
}
impl PhaseKind for Phase {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("only")
    }
    fn is_skippable(&self, _: &SkipKind) -> bool {
        false
    }
}

/// Free-form milestone id for the overflow test, which needs 1500+
/// distinct milestones and cannot rely on a closed enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
struct Milestone(CompactString);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Gate {}
impl knotch_kernel::GateKind for Gate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

#[derive(Debug, Clone, Copy)]
struct Wf;
const PHASES: [Phase; 1] = [Phase::Only];
impl WorkflowKind for Wf {
    type Phase = Phase;
    type Milestone = Milestone;
    type Gate = Gate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("filerepo-subscribe-test")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
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
    repo.append(
        &unit,
        vec![p(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .expect("seed");

    let mut stream = repo.subscribe(&unit, SubscribeMode::LiveOnly).await.expect("sub");
    let repo_cloned = repo.clone();
    let unit_cloned = unit.clone();
    tokio::spawn(async move {
        repo_cloned
            .append(
                &unit_cloned,
                vec![p(EventBody::MilestoneShipped {
                    milestone: Milestone("alpha".into()),
                    commit: CommitRef::new("abc"),
                    commit_kind: CommitKind::Feat,
                    status: CommitStatus::Verified,
                })],
                AppendMode::BestEffort,
            )
            .await
            .expect("live append");
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
    repo.append(
        &unit,
        vec![p(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .expect("seed");

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
    let report = repo
        .append(
            &unit,
            vec![
                p(EventBody::UnitCreated { scope: Scope::Standard }),
                p(EventBody::MilestoneShipped {
                    milestone: Milestone("alpha".into()),
                    commit: CommitRef::new("abc"),
                    commit_kind: CommitKind::Feat,
                    status: CommitStatus::Verified,
                }),
            ],
            AppendMode::BestEffort,
        )
        .await
        .expect("seed");
    let first_id = report.accepted[0].id;

    let mut stream =
        repo.subscribe(&unit, SubscribeMode::FromEventId(first_id)).await.expect("sub");
    let evt = stream.next().await.expect("replay");
    match evt {
        SubscribeEvent::Event(e) => assert!(matches!(e.body, EventBody::MilestoneShipped { .. })),
        other => panic!("{other:?}"),
    }
}

// --- B2 — broadcast overflow regression -----------------------------------

/// The in-process broadcast channel is bounded at `BROADCAST_CAPACITY =
/// 1024`. Subscribers that fall further behind must surface
/// `SubscribeEvent::Lagged { skipped, next }` so adopters can re-
/// subscribe or re-load, never silently lose state.
///
/// This test appends `CAPACITY + 500` events without polling the
/// subscriber, then drains the stream and asserts the first non-event
/// frame is `Lagged { skipped > 0 }` and that event delivery resumes
/// afterward.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broadcast_overflow_surfaces_lagged_event() {
    const EXTRA: usize = 500;

    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("lag-test");

    repo.append(
        &unit,
        vec![p(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .expect("seed");

    // Subscribe in LiveOnly mode so history does not inflate the
    // backlog; we want to see exactly how many events overflow the
    // broadcast buffer after subscription.
    let mut stream = repo.subscribe(&unit, SubscribeMode::LiveOnly).await.expect("sub");

    // Capacity-exceeding burst in one batch — each proposal has a
    // unique milestone id (newtype over CompactString) so fingerprint
    // dedup and "milestone already shipped" preconditions never fire.
    // Batching in one `append` avoids per-event lock overhead, keeping
    // the test fast.
    let total = 1024 + EXTRA;
    let proposals: Vec<_> = (0..total)
        .map(|i| {
            p(EventBody::MilestoneShipped {
                milestone: Milestone(format!("m{i:06}").into()),
                commit: CommitRef::new(format!("sha{i:06}")),
                commit_kind: CommitKind::Feat,
                status: CommitStatus::Verified,
            })
        })
        .collect();
    repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("append batch");

    // Drain the stream. The first non-Event frame must be Lagged.
    let mut saw_lagged_skipped: Option<u64> = None;
    let mut events_after_lag = 0;
    let mut frames_seen = 0;
    while frames_seen < total + 1 {
        match tokio::time::timeout(std::time::Duration::from_millis(250), stream.next()).await {
            Ok(Some(SubscribeEvent::Event(_))) => {
                if saw_lagged_skipped.is_some() {
                    events_after_lag += 1;
                }
                frames_seen += 1;
            }
            Ok(Some(SubscribeEvent::Lagged { skipped, .. })) => {
                assert!(skipped > 0, "Lagged must report skipped > 0");
                saw_lagged_skipped = Some(skipped);
                frames_seen += 1;
            }
            Ok(None) => break,
            Err(_timeout) => break,
        }
    }

    let skipped = saw_lagged_skipped.expect("must see SubscribeEvent::Lagged after capacity miss");
    // We sent 1024 + 500 in one batch; capacity is 1024. The first
    // Lagged reports at least `EXTRA` skipped frames — subscribers
    // that never poll see the oldest events evicted by newer ones.
    assert!(
        skipped as usize >= EXTRA,
        "expected skipped >= {EXTRA}, got {skipped}",
    );
    assert!(
        events_after_lag > 0,
        "stream must continue delivering events after the Lagged signal",
    );
}
