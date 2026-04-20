#![allow(missing_docs)]

//! # P1-6 example: interactive observer with cancellation
//!
//! Observers produce `Vec<Proposal<W>>` from a snapshot — they are
//! pure proposers. But long-running observers (those that poll a
//! slow VCS, wait on a human, or chunk a large backlog) must respect
//! the `CancellationToken` in [`ObserveContext::cancel`]. The
//! reconciler cancels observers that exceed their soft timeout and
//! records an `ObserverFailure { observer, source: Cancelled }`.
//!
//! This example walks through:
//!
//! 1. An observer that "asks for input" by polling a
//!    [`tokio::sync::watch`] channel between cancellation checks.
//! 2. What happens when the reconciler-side timeout fires before the
//!    answer arrives.
//! 3. What happens when the answer lands in time.
//!
//! Run with `cargo run -p knotch-example-interactive-observer`.

use std::{sync::Arc, time::Duration};

use knotch_kernel::{
    Causation, Proposal,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, EventBody},
};
use knotch_observer::{
    Observer,
    context::ObserveContext,
    error::ObserverError,
};
use knotch_workflow::{Knotch, KnotchPhase};
use tokio_util::sync::CancellationToken;

/// An observer that waits for a rationale string before emitting a
/// PhaseCompleted proposal. Polls the watch channel in a loop,
/// checking `ctx.cancel` between polls so cancellation lands
/// promptly.
struct AwaitInput {
    rx: tokio::sync::watch::Receiver<Option<String>>,
}

impl Observer<Knotch> for AwaitInput {
    fn name(&self) -> &'static str {
        "await-input"
    }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, Knotch>,
    ) -> Result<Vec<Proposal<Knotch>>, ObserverError> {
        let mut rx = self.rx.clone();
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(ObserverError::Cancelled {
                    name: self.name().into(),
                    elapsed_ms: 0,
                });
            }
            if let Some(_rationale) = rx.borrow().clone() {
                return Ok(vec![Proposal {
                    causation: Causation::new(
                        Source::Hook,
                        Principal::System {
                            service: "interactive-observer".into(),
                        },
                        Trigger::Observer {
                            name: self.name().into(),
                        },
                    ),
                    extension: (),
                    body: EventBody::PhaseCompleted {
                        phase: KnotchPhase::Build,
                        artifacts: ArtifactList::default(),
                    },
                    supersedes: None,
                }]);
            }
            tokio::select! {
                _ = ctx.cancel.cancelled() => {
                    return Err(ObserverError::Cancelled {
                        name: self.name().into(),
                        elapsed_ms: 0,
                    });
                }
                _ = rx.changed() => {}
            }
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(200)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use knotch_kernel::{Log, Repository, UnitId, time::Timestamp};
    use knotch_observer::context::ObserveBudget;
    use knotch_testing::InMemoryRepository;

    let repo = InMemoryRepository::<Knotch>::new(Knotch);
    let unit = UnitId::new("interactive-demo");
    let log: Arc<Log<Knotch>> = repo.load(&unit).await?;

    // Run 1 — no input arrives, expect cancellation.
    let (tx, rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let observer = AwaitInput { rx: rx.clone() };
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_for_task.cancel();
    });
    let cache = knotch_kernel::ResumeCache::default();
    let ctx = ObserveContext::<Knotch> {
        unit: &unit,
        log: log.clone(),
        head: "abc1234",
        cache: &cache,
        taken_at: Timestamp::now(),
        cancel: &cancel,
        budget: ObserveBudget::default(),
    };
    match observer.observe(&ctx).await {
        Err(ObserverError::Cancelled { name, .. }) => {
            println!("run 1: observer `{name}` cancelled cleanly — no proposal emitted");
        }
        other => anyhow::bail!("expected Cancelled, got {other:?}"),
    }

    // Run 2 — input arrives in time.
    let cancel = CancellationToken::new();
    let ctx = ObserveContext::<Knotch> {
        unit: &unit,
        log,
        head: "abc1234",
        cache: &cache,
        taken_at: Timestamp::now(),
        cancel: &cancel,
        budget: ObserveBudget::default(),
    };
    let observe_fut = observer.observe(&ctx);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(Some("user approved".into()));
    });
    let proposals = observe_fut.await?;
    println!(
        "run 2: observer returned {} proposal(s) after input arrived",
        proposals.len(),
    );
    Ok(())
}
