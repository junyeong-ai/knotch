//! VCS error taxonomy.

use std::path::PathBuf;

/// Errors surfaced by a `Vcs` adapter.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VcsError {
    /// Backend failed to open the repository.
    #[error("failed to open repository at {path:?}")]
    OpenRepository {
        /// Repository path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// HEAD is unresolvable (empty repo / detached in a bad state).
    #[error("HEAD is unresolvable")]
    HeadUnresolvable {
        /// Underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// Backend-specific failure.
    #[error("vcs backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}
