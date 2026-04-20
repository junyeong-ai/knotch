//! `PendingCommitObserver` — promotes `Pending` ships to `Verified`.
//!
//! Scans the log for `MilestoneShipped { status: Pending }` entries
//! whose commit has since become visible, and proposes a corresponding
//! `MilestoneVerified` event. The observer is idempotent — once the
//! verified event lands, future passes skip that (milestone, commit)
//! pair.

use std::{marker::PhantomData, sync::Arc};

use compact_str::CompactString;
use knotch_kernel::{
    Causation, CommitStatus, MilestoneKind, Proposal, WorkflowKind,
    causation::{Source, Trigger},
    event::EventBody,
    project::effective_events,
};
use knotch_vcs::Vcs;

use crate::{ObserveContext, Observer, ObserverError};

/// Observer that promotes pending ships.
pub struct PendingCommitObserver<V, W: WorkflowKind> {
    vcs: Arc<V>,
    _marker: PhantomData<fn() -> W>,
}

impl<V: Vcs, W: WorkflowKind> PendingCommitObserver<V, W> {
    /// Construct the observer. The observer reads the unit id from
    /// `ObserveContext` at call time — keeping the type workflow-wide
    /// rather than unit-scoped so one instance can serve many units.
    pub fn new(vcs: Arc<V>) -> Self {
        Self { vcs, _marker: PhantomData }
    }
}

impl<V, W> Observer<W> for PendingCommitObserver<V, W>
where
    V: Vcs,
    W: WorkflowKind,
{
    fn name(&self) -> &str {
        "pending-commit"
    }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> Result<Vec<Proposal<W>>, ObserverError> {
        let effective = effective_events::<W>(ctx.log.as_ref());

        // Collect pending (milestone, commit) pairs in append order.
        let mut pending: Vec<(W::Milestone, knotch_kernel::event::CommitRef)> = Vec::new();
        // Track already-verified pairs to suppress duplicates.
        let mut verified: Vec<(String, String)> = Vec::new();

        for evt in &effective {
            match &evt.body {
                EventBody::MilestoneShipped {
                    milestone,
                    commit,
                    status: CommitStatus::Pending,
                    ..
                } => {
                    pending.push((milestone.clone(), commit.clone()));
                }
                EventBody::MilestoneVerified { milestone, commit } => {
                    let id: String = MilestoneKind::id(milestone).into_owned();
                    verified.push((id, commit.as_str().to_owned()));
                }
                _ => {}
            }
        }

        let mut out = Vec::new();
        for (milestone, commit) in pending {
            if ctx.cancel.is_cancelled() {
                return Err(ObserverError::Cancelled {
                    name: CompactString::from("pending-commit"),
                    elapsed_ms: 0,
                });
            }
            let key = (MilestoneKind::id(&milestone).into_owned(), commit.as_str().to_owned());
            if verified.contains(&key) {
                continue;
            }
            let status = self
                .vcs
                .verify_commit(&commit)
                .await
                .map_err(|e| ObserverError::Vcs(Box::new(e)))?;
            if !matches!(status, CommitStatus::Verified) {
                continue;
            }
            let causation = Causation::new(
                Source::Observer,
                Trigger::Observer { name: CompactString::from("pending-commit") },
            );
            out.push(Proposal {
                body: EventBody::MilestoneVerified { milestone, commit },
                causation,
                extension: serde_json::from_value(serde_json::Value::Null)
                    .expect("extension must deserialize from null"),
                supersedes: None,
            });
            if out.len() >= ctx.budget.max_proposals {
                return Err(ObserverError::BudgetExceeded {
                    name: CompactString::from("pending-commit"),
                    limit: ctx.budget.max_proposals,
                });
            }
        }
        Ok(out)
    }
}
