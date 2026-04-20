//! Reconcile error taxonomy.

use knotch_kernel::RepositoryError;

/// Errors returned by `Reconciler::reconcile`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReconcileError {
    /// Repository load or append failed.
    #[error("repository failure")]
    Repository(#[source] RepositoryError),
    /// An observer task panicked or was aborted.
    #[error("observer task join error: {0}")]
    JoinError(String),
}
