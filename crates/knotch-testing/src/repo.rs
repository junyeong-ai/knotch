//! In-memory `Repository<W>` for tests.
//!
//! The adapter enforces the kernel-level invariants that all concrete
//! repositories share:
//!
//! - Fingerprint dedup (replayed proposals surface as
//!   `AppendReport::rejected` with reason "duplicate").
//! - Monotonic event timestamps.
//! - Single-writer discipline via a `tokio::sync::Mutex` per unit.
//! - Subscribe streams via `tokio::sync::broadcast` — backpressure
//!   surfaces as `SubscribeEvent::Lagged` rather than silent loss.
//!
//! Adapter-level concerns (atomic-write, cross-process locking,
//! corruption recovery) belong to the file-system repository.

use std::sync::Arc;

use async_stream::try_stream;
use dashmap::DashMap;
use futures::{StreamExt as _, stream};
use jiff::Timestamp;
use knotch_kernel::{
    AppendMode, AppendReport, Event, EventId, ExtensionKind as _, Fingerprint, Log, Proposal,
    RepositoryError, UnitId, WorkflowKind, fingerprint_proposal,
    event::{RejectedProposal, SubscribeEvent, SubscribeMode},
    repository::{PinStream, Repository, ResumeCache},
};
use tokio::sync::{Mutex as AsyncMutex, broadcast};

/// Default broadcast buffer per unit. Subscribers that fall this far
/// behind receive `SubscribeEvent::Lagged`.
pub const DEFAULT_BROADCAST_CAPACITY: usize = 1024;

type UnitState<W> = Arc<AsyncMutex<UnitStateInner<W>>>;

struct UnitStateInner<W: WorkflowKind> {
    events: Vec<Event<W>>,
    cache: ResumeCache,
    fingerprints: Vec<Fingerprint>,
    broadcast: broadcast::Sender<Event<W>>,
}

impl<W: WorkflowKind> UnitStateInner<W> {
    fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { events: Vec::new(), cache: ResumeCache::new(), fingerprints: Vec::new(), broadcast: tx }
    }
}

/// In-memory repository — cheap to clone; all clones share state.
pub struct InMemoryRepository<W: WorkflowKind> {
    workflow: W,
    units: Arc<DashMap<UnitId, UnitState<W>>>,
    broadcast_capacity: usize,
}

impl<W: WorkflowKind> Clone for InMemoryRepository<W> {
    fn clone(&self) -> Self {
        Self {
            workflow: self.workflow.clone(),
            units: self.units.clone(),
            broadcast_capacity: self.broadcast_capacity,
        }
    }
}

impl<W: WorkflowKind + Default> Default for InMemoryRepository<W> {
    fn default() -> Self {
        Self::new(W::default())
    }
}

impl<W: WorkflowKind> InMemoryRepository<W> {
    /// Construct a fresh repository for the given workflow with the
    /// default broadcast capacity.
    #[must_use]
    pub fn new(workflow: W) -> Self {
        Self::with_capacity(workflow, DEFAULT_BROADCAST_CAPACITY)
    }

    /// Construct with a custom broadcast buffer size.
    #[must_use]
    pub fn with_capacity(workflow: W, capacity: usize) -> Self {
        Self {
            workflow,
            units: Arc::new(DashMap::new()),
            broadcast_capacity: capacity.max(1),
        }
    }

    /// Borrow the workflow instance this repository was built with.
    #[must_use]
    pub fn workflow(&self) -> &W {
        &self.workflow
    }

    fn unit_state(&self, unit: &UnitId) -> UnitState<W> {
        let capacity = self.broadcast_capacity;
        self.units
            .entry(unit.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(UnitStateInner::new(capacity))))
            .clone()
    }
}

impl<W: WorkflowKind> Repository<W> for InMemoryRepository<W> {
    fn workflow(&self) -> &W {
        &self.workflow
    }

    async fn append(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
    ) -> Result<AppendReport<W>, RepositoryError> {
        let state = self.unit_state(unit);
        let mut inner = state.lock().await;
        let accepted_before = inner.events.len();
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();

        for proposal in proposals {
            let fingerprint = fingerprint_proposal(&self.workflow, &proposal)
                .map_err(RepositoryError::Codec)?;
            if inner.fingerprints.contains(&fingerprint) {
                rejected.push(RejectedProposal { proposal, reason: "duplicate".into() });
                continue;
            }
            // Body + extension preconditions against the working log.
            let working_log =
                knotch_kernel::Log::from_events(unit.clone(), inner.events.clone());
            let ctx = knotch_kernel::precondition::AppendContext::<W>::new(&self.workflow, &working_log);
            if let Err(err) = proposal.body.check_precondition(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    inner.events.truncate(accepted_before);
                    inner.fingerprints.truncate(accepted_before);
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal {
                    proposal,
                    reason: err.to_string().into(),
                });
                continue;
            }
            if let Err(err) = proposal.extension.check_extension::<W>(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    inner.events.truncate(accepted_before);
                    inner.fingerprints.truncate(accepted_before);
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal {
                    proposal,
                    reason: err.to_string().into(),
                });
                continue;
            }
            let at = Timestamp::now();
            let last_at = inner.events.last().map(|e| e.at);
            if let Some(last_at) = last_at {
                if at < last_at {
                    if matches!(mode, AppendMode::AllOrNothing) {
                        inner.events.truncate(accepted_before);
                        inner.fingerprints.truncate(accepted_before);
                        return Err(RepositoryError::NonMonotonic { attempted: at, last: last_at });
                    }
                    rejected.push(RejectedProposal { proposal, reason: "non-monotonic".into() });
                    continue;
                }
            }
            let event = Event {
                id: EventId::new_v7(),
                at,
                causation: proposal.causation.clone(),
                extension: proposal.extension.clone(),
                body: proposal.body.clone(),
                supersedes: proposal.supersedes,
            };
            inner.events.push(event.clone());
            inner.fingerprints.push(fingerprint);
            accepted.push(event);
        }

        if matches!(mode, AppendMode::AllOrNothing) && !rejected.is_empty() {
            inner.events.truncate(accepted_before);
            inner.fingerprints.truncate(accepted_before);
            return Ok(AppendReport { accepted: Vec::new(), rejected });
        }

        // Fan out accepted events to subscribers. `send` only fails
        // when there are zero receivers — that's fine.
        for event in &accepted {
            let _ = inner.broadcast.send(event.clone());
        }

        Ok(AppendReport { accepted, rejected })
    }

    async fn load(&self, unit: &UnitId) -> Result<Arc<Log<W>>, RepositoryError> {
        let state = self.unit_state(unit);
        let inner = state.lock().await;
        Ok(Arc::new(Log::from_events(unit.clone(), inner.events.clone())))
    }

    async fn subscribe(
        &self,
        unit: &UnitId,
        mode: SubscribeMode,
    ) -> Result<PinStream<SubscribeEvent<W>>, RepositoryError> {
        let state = self.unit_state(unit);
        let (rx, replay) = {
            let inner = state.lock().await;
            let rx = inner.broadcast.subscribe();
            let replay = match mode {
                SubscribeMode::LiveOnly => Vec::new(),
                SubscribeMode::FromBeginning => inner.events.clone(),
                SubscribeMode::FromEventId(anchor) => {
                    let idx = inner
                        .events
                        .iter()
                        .position(|e| e.id == anchor)
                        .map_or(0, |i| i + 1);
                    inner.events[idx..].to_vec()
                }
                _ => Vec::new(),
            };
            (rx, replay)
        };

        let stream = try_stream! {
            for evt in replay {
                yield SubscribeEvent::Event(Box::new(evt));
            }
            let mut rx = rx;
            loop {
                match rx.recv().await {
                    Ok(evt) => yield SubscribeEvent::Event(Box::new(evt)),
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Surface a synthetic Lagged frame so the
                        // subscriber can resync by calling `load`.
                        match rx.recv().await {
                            Ok(evt) => {
                                let id = evt.id;
                                yield SubscribeEvent::Lagged { skipped, next: id };
                                yield SubscribeEvent::Event(Box::new(evt));
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        }
                    }
                }
            }
        };
        // `try_stream!` yields Results; unwrap into plain items since
        // InMemoryRepository's subscribe cannot fail after the initial
        // snapshot.
        Ok(Box::pin(stream.map(|r: Result<_, std::convert::Infallible>| match r {
            Ok(item) => item,
            Err(e) => match e {},
        })))
    }

    fn list_units(&self) -> PinStream<Result<UnitId, RepositoryError>> {
        let mut ids: Vec<_> = self.units.iter().map(|e| e.key().clone()).collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Box::pin(stream::iter(ids.into_iter().map(Ok)))
    }

    async fn with_cache(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
        mutate_cache: knotch_kernel::repository::CacheMutator,
    ) -> Result<AppendReport<W>, RepositoryError> {
        let state = self.unit_state(unit);
        let mut inner = state.lock().await;
        let accepted_before = inner.events.len();
        // Snapshot-then-commit: `inner.cache` is untouched until every
        // precondition + timestamp check succeeds. If any step fails
        // under `AllOrNothing`, `inner.cache` remains the pre-mutation
        // value — callers cannot observe the mutator's partial effect.
        let mut working_cache = inner.cache.clone();
        mutate_cache(&mut working_cache);

        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        for proposal in proposals {
            let fingerprint = fingerprint_proposal(&self.workflow, &proposal)
                .map_err(RepositoryError::Codec)?;
            if inner.fingerprints.contains(&fingerprint) {
                rejected.push(RejectedProposal { proposal, reason: "duplicate".into() });
                continue;
            }
            let working_log =
                knotch_kernel::Log::from_events(unit.clone(), inner.events.clone());
            let ctx = knotch_kernel::precondition::AppendContext::<W>::new(&self.workflow, &working_log);
            if let Err(err) = proposal.body.check_precondition(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    inner.events.truncate(accepted_before);
                    inner.fingerprints.truncate(accepted_before);
                    // `inner.cache` is left pristine — we only
                    // mutated `working_cache`.
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal { proposal, reason: err.to_string().into() });
                continue;
            }
            if let Err(err) = proposal.extension.check_extension::<W>(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    inner.events.truncate(accepted_before);
                    inner.fingerprints.truncate(accepted_before);
                    // `inner.cache` is left pristine — we only
                    // mutated `working_cache`.
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal { proposal, reason: err.to_string().into() });
                continue;
            }
            let at = Timestamp::now();
            let last_at = inner.events.last().map(|e| e.at);
            if let Some(last_at) = last_at {
                if at < last_at {
                    if matches!(mode, AppendMode::AllOrNothing) {
                        inner.events.truncate(accepted_before);
                        inner.fingerprints.truncate(accepted_before);
                        return Err(RepositoryError::NonMonotonic { attempted: at, last: last_at });
                    }
                    rejected.push(RejectedProposal { proposal, reason: "non-monotonic".into() });
                    continue;
                }
            }
            let event = Event {
                id: EventId::new_v7(),
                at,
                causation: proposal.causation.clone(),
                extension: proposal.extension.clone(),
                body: proposal.body.clone(),
                supersedes: proposal.supersedes,
            };
            inner.events.push(event.clone());
            inner.fingerprints.push(fingerprint);
            accepted.push(event);
        }

        if matches!(mode, AppendMode::AllOrNothing) && !rejected.is_empty() {
            inner.events.truncate(accepted_before);
            inner.fingerprints.truncate(accepted_before);
            // `inner.cache` is already the pre-mutation value.
            return Ok(AppendReport { accepted: Vec::new(), rejected });
        }

        // Commit both sides atomically under the same lock.
        for event in &accepted {
            let _ = inner.broadcast.send(event.clone());
        }
        inner.cache = working_cache;

        Ok(AppendReport { accepted, rejected })
    }
}

