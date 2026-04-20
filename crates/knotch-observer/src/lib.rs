//! `Observer<W>` trait and first-party observers.
//!
//! An observer proposes events from external state (git log, filesystem,
//! spec frontmatter, shell-command output). Observers are **pure
//! proposers** — they never mutate the repository; the Reconciler
//! composes their proposals into a single ordered batch and submits it
//! to `Repository::append`.
//!
//! Observers are idempotent by contract: running the same observer on
//! the same inputs must produce the same proposals. Fingerprints on
//! each event ensure that replayed proposals are rejected as
//! duplicates.

pub mod artifact;
pub mod context;
pub mod error;
pub mod git_log;
pub mod pending_commit;
pub mod subprocess;

pub use self::{
    artifact::ArtifactObserver,
    context::{FsView, ObserveBudget, ObserveContext, StdFsView},
    error::ObserverError,
    git_log::GitLogObserver,
    pending_commit::PendingCommitObserver,
    subprocess::{ObserverManifest, SubprocessError, SubprocessObserver},
};

use std::{future::Future, pin::Pin, time::Duration};

use knotch_kernel::{Proposal, WorkflowKind};

/// `Observer` port. Each observer is responsible for a single source
/// of truth; the Reconciler composes many observers in parallel.
///
/// The trait returns a concrete `impl Future` so implementations use
/// native async. For dyn storage the Reconciler uses
/// [`DynObserver`], which has a blanket impl for every `Observer`.
pub trait Observer<W: WorkflowKind>: Send + Sync + 'static {
    /// Stable observer name, used in `Causation::Trigger::Observer`
    /// and as a deterministic merge key. The lifetime is tied to
    /// `&self` so subprocess-backed observers can carry a runtime
    /// name (loaded from `knotch.toml`) without leaking it.
    fn name(&self) -> &str;

    /// Produce a batch of proposals from the current observation
    /// context.
    ///
    /// # Errors
    /// Returns `ObserverError` for backend or cancellation failures.
    fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> impl Future<Output = Result<Vec<Proposal<W>>, ObserverError>> + Send + 'ctx;

    /// Per-observer soft timeout.
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Type-erased observe future used by [`DynObserver::observe_boxed`].
pub type BoxObserveFuture<'ctx, W> =
    Pin<Box<dyn Future<Output = Result<Vec<Proposal<W>>, ObserverError>> + Send + 'ctx>>;

/// Dyn-compatible variant of [`Observer`]. Implemented automatically
/// for every `Observer<W>` via a blanket impl so callers can store
/// `Arc<dyn DynObserver<W>>` vectors.
pub trait DynObserver<W: WorkflowKind>: Send + Sync + 'static {
    /// Stable observer name.
    fn name(&self) -> &str;

    /// Type-erased observe call.
    fn observe_boxed<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> BoxObserveFuture<'ctx, W>;

    /// Per-observer soft timeout.
    fn timeout(&self) -> Duration;
}

impl<W, O> DynObserver<W> for O
where
    W: WorkflowKind,
    O: Observer<W>,
{
    fn name(&self) -> &str {
        Observer::<W>::name(self)
    }

    fn observe_boxed<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> BoxObserveFuture<'ctx, W> {
        Box::pin(Observer::<W>::observe(self, ctx))
    }

    fn timeout(&self) -> Duration {
        Observer::<W>::timeout(self)
    }
}
