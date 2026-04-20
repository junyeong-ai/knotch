//! Observer cancellation proof — a slow observer exits promptly
//! when its `CancellationToken` is tripped.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc, time::Duration};

use knotch_kernel::{
    Log, PhaseKind, Proposal, Scope, UnitId, WorkflowKind,
    event::{SkipKind},
};
use knotch_observer::{ObserveBudget, ObserveContext, Observer, ObserverError};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum Phase { Only }
impl PhaseKind for Phase {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed("only") }
    fn is_skippable(&self, _: &SkipKind) -> bool { false }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct M(String);
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed(&self.0) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct G(String);
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed(&self.0) }
}

#[derive(Debug, Clone, Copy)]
struct Wf;
const PHASES: [Phase; 1] = [Phase::Only];
impl WorkflowKind for Wf {
    type Phase = Phase;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("cancel-fixture") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

/// Observer that loops forever, polling the cancellation token on
/// every iteration. A correctly-implemented observer wakes up on
/// token trip and returns `ObserverError::Cancelled` promptly.
struct SlowObserver;

impl Observer<Wf> for SlowObserver {
    fn name(&self) -> &'static str { "slow" }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, Wf>,
    ) -> Result<Vec<Proposal<Wf>>, ObserverError> {
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(ObserverError::Cancelled {
                    name: "slow".into(),
                    elapsed_ms: 0,
                });
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[tokio::test]
async fn cancellation_token_stops_an_inflight_observer() {
    let unit = UnitId::new("u");
    let log: Arc<Log<Wf>> = Arc::new(Log::empty(unit.clone()));
    let cache = knotch_kernel::repository::ResumeCache::new();
    let cancel = CancellationToken::new();
    let ctx = ObserveContext::<Wf> {
        unit: &unit,
        log: log.clone(),
        head: "",
        cache: &cache,
        taken_at: jiff::Timestamp::now(),
        cancel: &cancel,
        budget: ObserveBudget::default(),
    };

    let observer = SlowObserver;
    let cancel_after = cancel.clone();
    let trip = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_after.cancel();
    });

    let start = tokio::time::Instant::now();
    let result = observer.observe(&ctx).await;
    trip.await.expect("join");
    let elapsed = start.elapsed();

    assert!(matches!(result, Err(ObserverError::Cancelled { .. })));
    assert!(
        elapsed < Duration::from_millis(500),
        "observer ran for {elapsed:?} — cancellation didn't short-circuit",
    );
}
