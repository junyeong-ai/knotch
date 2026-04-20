//! Observer composition and deterministic merge.
//!
//! Flow:
//!
//! 1. Acquire a snapshot of the unit's log via `Repository::load`.
//! 2. Run every registered `Observer` against an `ObserveContext`
//!    built from that snapshot. Observers execute on a shared
//!    `tokio::task::JoinSet` so I/O-bound observers overlap.
//! 3. Collect proposals, sort them deterministically by
//!    `(observer_name, body_kind_tag, fingerprint)`.
//! 4. Submit the sorted batch to `Repository::append` with
//!    `AppendMode::BestEffort` so rejections (duplicates, ordering)
//!    don't block the batch. `AllOrNothing` is reserved for
//!    preset-level reconciles that need atomicity.

use std::sync::Arc;

use knotch_kernel::{
    AppendMode, AppendReport, Repository, UnitId, WorkflowKind,
    event::{EventBody, Proposal, RejectedProposal},
};
use knotch_observer::{DynObserver, ObserveBudget, ObserveContext, ObserverError};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

mod error;

pub use self::error::ReconcileError;

/// Reconciler — composes observers against a repository.
pub struct Reconciler<W: WorkflowKind, R: Repository<W>> {
    repo: Arc<R>,
    observers: Vec<Arc<dyn DynObserver<W>>>,
    append_mode: AppendMode,
    default_budget: ObserveBudget,
}

impl<W: WorkflowKind, R: Repository<W>> Reconciler<W, R> {
    /// Start a builder.
    #[must_use]
    pub fn builder(repo: Arc<R>) -> ReconcilerBuilder<W, R> {
        ReconcilerBuilder {
            repo,
            observers: Vec::new(),
            append_mode: AppendMode::BestEffort,
            default_budget: ObserveBudget::default(),
        }
    }

    /// Run a reconcile pass against `unit`.
    ///
    /// # Errors
    /// Surfaces repository errors (load or append) and the first
    /// catastrophic observer error (observer-level errors that don't
    /// propose anything are collected in `ReconcileReport::observer_errors`).
    #[instrument(skip_all, fields(unit = %unit))]
    pub async fn reconcile(&self, unit: &UnitId) -> Result<ReconcileReport<W>, ReconcileError> {
        let log = self.repo.load(unit).await.map_err(ReconcileError::Repository)?;
        let cache = knotch_kernel::repository::ResumeCache::new();
        let cancel = CancellationToken::new();
        let taken_at = jiff::Timestamp::now();
        let head = log
            .events()
            .iter()
            .rev()
            .find_map(|e| match &e.body {
                EventBody::MilestoneShipped { commit, .. } => Some(commit.as_str().to_owned()),
                _ => None,
            })
            .unwrap_or_default();

        let mut join = JoinSet::new();
        for observer in &self.observers {
            let obs = observer.clone();
            let log = log.clone();
            let head = head.clone();
            let cancel_child = cancel.child_token();
            let cache = cache.clone_for_observer();
            let budget = self.default_budget;
            let unit = unit.clone();
            join.spawn(async move {
                let ctx = ObserveContext::<W> {
                    unit: &unit,
                    log,
                    head: &head,
                    cache: &cache,
                    taken_at,
                    cancel: &cancel_child,
                    budget,
                };
                let timeout = obs.timeout();
                let name: String = obs.name().to_owned();
                let result = tokio::time::timeout(timeout, obs.observe_boxed(&ctx)).await;
                (name, result)
            });
        }

        let mut all_proposals: Vec<(String, Proposal<W>)> = Vec::new();
        let mut observer_errors: Vec<ObserverFailure> = Vec::new();
        while let Some(joined) = join.join_next().await {
            let (name, result) = joined.map_err(|e| ReconcileError::JoinError(e.to_string()))?;
            match result {
                Ok(Ok(proposals)) => {
                    for p in proposals {
                        all_proposals.push((name.clone(), p));
                    }
                }
                Ok(Err(err)) => observer_errors.push(ObserverFailure {
                    observer: name,
                    source: err,
                }),
                Err(_elapsed) => observer_errors.push(ObserverFailure {
                    observer: name.clone(),
                    source: ObserverError::Cancelled {
                        name: name.into(),
                        elapsed_ms: 0,
                    },
                }),
            }
        }

        // Deterministic order: (observer_name, kind_ordinal, kind_tag).
        // Two proposals from the same observer carrying the same body
        // kind serialize to the same fingerprint and are therefore
        // deduped by the Repository — the pair `(observer_name, body
        // kind)` is sufficient for deterministic ordering and no
        // body-debug tertiary is required. Cf. constitution §IX and
        // `.claude/rules/fingerprint.md`.
        all_proposals.sort_by(|(an, ap), (bn, bp)| {
            an.cmp(bn)
                .then_with(|| kind_tag(&ap.body).cmp(&kind_tag(&bp.body)))
        });

        let proposals: Vec<Proposal<W>> =
            all_proposals.into_iter().map(|(_, p)| p).collect();

        let append_report = self
            .repo
            .append(unit, proposals, self.append_mode)
            .await
            .map_err(ReconcileError::Repository)?;

        Ok(ReconcileReport { append: append_report, observer_errors })
    }
}

/// Builder for [`Reconciler`].
pub struct ReconcilerBuilder<W: WorkflowKind, R: Repository<W>> {
    repo: Arc<R>,
    observers: Vec<Arc<dyn DynObserver<W>>>,
    append_mode: AppendMode,
    default_budget: ObserveBudget,
}

impl<W: WorkflowKind, R: Repository<W>> ReconcilerBuilder<W, R> {
    /// Register an observer.
    #[must_use]
    pub fn observer(mut self, observer: Arc<dyn DynObserver<W>>) -> Self {
        self.observers.push(observer);
        self
    }

    /// Override the default observer budget.
    #[must_use]
    pub fn budget(mut self, budget: ObserveBudget) -> Self {
        self.default_budget = budget;
        self
    }

    /// Override the append mode (defaults to `BestEffort`).
    #[must_use]
    pub fn append_mode(mut self, mode: AppendMode) -> Self {
        self.append_mode = mode;
        self
    }

    /// Finalize into a `Reconciler`.
    #[must_use]
    pub fn build(self) -> Reconciler<W, R> {
        Reconciler {
            repo: self.repo,
            observers: self.observers,
            append_mode: self.append_mode,
            default_budget: self.default_budget,
        }
    }
}

/// Aggregated result of a reconcile pass.
#[derive(Debug)]
pub struct ReconcileReport<W: WorkflowKind> {
    /// Outcome of the repository append.
    pub append: AppendReport<W>,
    /// Per-observer failures that produced zero proposals.
    pub observer_errors: Vec<ObserverFailure>,
}

impl<W: WorkflowKind> ReconcileReport<W> {
    /// Were any new events accepted this pass?
    #[must_use]
    pub fn accepted_any(&self) -> bool {
        !self.append.accepted.is_empty()
    }

    /// Number of rejected proposals (duplicates, non-monotonic, etc.).
    #[must_use]
    pub fn rejected_count(&self) -> usize {
        self.append.rejected.len()
    }

    /// All rejection details.
    #[must_use]
    pub fn rejected(&self) -> &[RejectedProposal<W>] {
        &self.append.rejected
    }
}

/// Per-observer failure record.
#[derive(Debug)]
pub struct ObserverFailure {
    /// Observer that failed.
    pub observer: String,
    /// Underlying error — follows the workspace convention of naming
    /// the nested error field `source` (see the charter's appendix A).
    pub source: ObserverError,
}

/// Stable sort key — `(ordinal, tag)` composed without duplicating
/// the tag enumeration. Delegates to `EventBody::kind_ordinal` /
/// `kind_tag`, both single source of truth in the kernel.
fn kind_tag<W: WorkflowKind>(body: &EventBody<W>) -> String {
    format!("{:02}.{}", body.kind_ordinal(), body.kind_tag())
}

// Observers receive a fresh empty `ResumeCache` per reconcile call.
// `ResumeCache` is not `Clone`, and observers never mutate it; they
// read a per-unit snapshot. Handing out a fresh instance is the
// simplest way to satisfy the observer context's cache field
// without exposing interior mutability. Adapters that persist
// resume-cache state do so through `Repository::with_cache`, which
// runs on the write path, not the observer-read path.
trait CloneForObserver {
    fn clone_for_observer(&self) -> Self;
}

impl CloneForObserver for knotch_kernel::repository::ResumeCache {
    fn clone_for_observer(&self) -> Self {
        Self::new()
    }
}
