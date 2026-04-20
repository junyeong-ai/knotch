//! Append-time preconditions.
//!
//! Repository implementations evaluate `EventBody::check_precondition`
//! inside their lock window, against a freshly-loaded `Log<W>`.
//! This module supplies:
//!
//! - [`AppendContext`] — the read-only snapshot observers get.
//! - [`VerifyCommit`] — optional VCS probe (for `MilestoneShipped` /
//!   `MilestoneReverted`); pass `None` in pure-kernel tests.
//! - [`ArtifactCheck`] — optional filesystem probe (for
//!   `PhaseCompleted`'s artifact contract).
//!
//! Per-body dispatch is implemented as an **inherent method** on
//! `EventBody<W>` in `event.rs` — extension-contributed preconditions
//! go through [`ExtensionKind::check_extension`](crate::ExtensionKind).

use std::path::Path;

use crate::{
    error::PreconditionError,
    event::CommitStatus,
    log::Log,
    workflow::WorkflowKind,
};

/// Read-only snapshot handed to every precondition evaluation.
///
/// Holds the log snapshot, the unit identity, wall-clock timestamp,
/// and two **optional** external-state probes. Adapters that can
/// answer VCS / filesystem questions supply concrete implementations;
/// pure in-memory tests pass `None` and the per-variant precondition
/// degrades to log-only checks.
pub struct AppendContext<'a, W: WorkflowKind> {
    /// Workflow instance carrying the shape (phases, gates,
    /// terminal statuses, rationale floor, …). Preconditions consult
    /// this rather than calling associated functions — required
    /// because runtime-configurable workflows (e.g. `ConfigWorkflow`)
    /// carry their shape as data.
    pub workflow: &'a W,
    /// Authoritative log snapshot taken under the Repository's lock.
    pub log: &'a Log<W>,
    /// Optional VCS verifier. `None` means "skip external-state
    /// checks that depend on VCS visibility".
    pub vcs: Option<&'a dyn VerifyCommit>,
    /// Optional filesystem view. `None` means "skip artifact-existence
    /// checks" — the Repository accepts the event and trusts the
    /// caller's artifact list.
    pub fs: Option<&'a dyn ArtifactCheck>,
}

impl<'a, W: WorkflowKind> AppendContext<'a, W> {
    /// Construct a log-only context — no VCS probe, no filesystem
    /// probe. Used by tests and by the kernel's trivial repository
    /// paths.
    #[must_use]
    pub fn new(workflow: &'a W, log: &'a Log<W>) -> Self {
        Self { workflow, log, vcs: None, fs: None }
    }

    /// Attach a VCS verifier.
    #[must_use]
    pub fn with_vcs(mut self, vcs: &'a dyn VerifyCommit) -> Self {
        self.vcs = Some(vcs);
        self
    }

    /// Attach a filesystem view.
    #[must_use]
    pub fn with_fs(mut self, fs: &'a dyn ArtifactCheck) -> Self {
        self.fs = Some(fs);
        self
    }
}

/// Synchronous VCS probe — used by `MilestoneShipped` /
/// `MilestoneReverted` preconditions to resolve `CommitStatus` before
/// accepting an append.
pub trait VerifyCommit: Send + Sync {
    /// Return the visibility status of a commit at the current
    /// Repository snapshot.
    ///
    /// # Errors
    /// Return `PreconditionError::CommitUnverifiable` when the VCS
    /// backend fails in a way that should block the append.
    fn verify(
        &self,
        sha: &crate::event::CommitRef,
    ) -> Result<CommitStatus, PreconditionError>;
}

/// Synchronous filesystem probe — used by `PhaseCompleted` to verify
/// that artifact paths exist at append time.
pub trait ArtifactCheck: Send + Sync {
    /// Does `path` exist?
    fn exists(&self, path: &Path) -> bool;
}
