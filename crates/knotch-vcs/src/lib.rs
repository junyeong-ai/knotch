//! Version-control-system adapter trait and built-in `gix`
//! implementation.
//!
//! The `Vcs` trait is the port; `GixVcs` is the default adapter; the
//! in-memory substitute for tests lives in `knotch-testing::vcs`.
//!
//! `CommitStatus::Pending` models commits that exist locally but are
//! not yet visible to the verifier (e.g. remote-only in a distributed
//! team); a later reconcile pass promotes them to `Verified` by
//! emitting a `MilestoneVerified` event.

pub mod commit;
pub mod error;
pub mod parse;

mod gix_vcs;

pub use self::{
    commit::{Commit, CommitStatus, ParsedCommit, RevertLink, Watermark},
    error::VcsError,
    gix_vcs::GixVcs,
};

use std::future::Future;

use knotch_kernel::event::{CommitKind, CommitRef};

/// Filter applied by `Vcs::log_since`.
#[derive(Debug, Clone, Default)]
pub struct CommitFilter {
    /// Include only these commit kinds, or all if empty.
    pub kinds: Vec<CommitKind>,
    /// Cap on the number of commits returned. `None` = unbounded.
    pub limit: Option<usize>,
}

/// Version-control-system port.
pub trait Vcs: Send + Sync + 'static {
    /// Verify whether a commit is visible in the adapter's view.
    ///
    /// # Errors
    /// Returns `VcsError` on backend failure. A non-visible commit is
    /// not an error — it surfaces as `CommitStatus::{Pending, Missing}`.
    fn verify_commit(
        &self,
        sha: &CommitRef,
    ) -> impl Future<Output = Result<CommitStatus, VcsError>> + Send;

    /// Walk commits between `since` (exclusive) and HEAD (inclusive).
    ///
    /// `since = None` walks every reachable commit from HEAD. Returned
    /// order is newest-first (ancestors later in the vector).
    ///
    /// # Errors
    /// Returns `VcsError` on backend failure.
    fn log_since(
        &self,
        since: Option<&CommitRef>,
        filter: &CommitFilter,
    ) -> impl Future<Output = Result<Vec<Commit>, VcsError>> + Send;

    /// Return the current HEAD ref.
    ///
    /// # Errors
    /// Returns `VcsError` on backend failure or detached HEAD without
    /// any commit history.
    fn current_head(&self) -> impl Future<Output = Result<CommitRef, VcsError>> + Send;

    /// Return the observer-watermark cursor for this repository.
    /// Callers typically compare this against the `ResumeCache` to
    /// decide what's new since the last reconcile.
    ///
    /// # Errors
    /// Returns `VcsError` on backend failure.
    fn log_watermark(&self) -> impl Future<Output = Result<Watermark, VcsError>> + Send;

    /// Detect revert linkage for a commit. Default implementation
    /// reads the `reverts` hint produced by the parser.
    fn detect_revert(&self, commit: &ParsedCommit) -> Option<RevertLink> {
        commit.reverts.as_ref().map(|target| RevertLink {
            original: target.clone(),
            revert: commit.sha.clone(),
        })
    }
}
