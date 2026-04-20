//! Storage error taxonomy.

use std::{io, path::PathBuf};

/// Errors surfaced by a `Storage` adapter.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Underlying I/O failure.
    #[error("storage I/O error at {path:?}")]
    Io {
        /// Path that failed, if known.
        path: Option<PathBuf>,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Optimistic-CAS mismatch — another writer extended the log
    /// between our load and our append.
    #[error("log was mutated by another writer (had {on_disk} lines, expected {expected})")]
    LogMutated {
        /// The line count the caller passed as `expected_len`.
        expected: u64,
        /// The actual line count the adapter observed.
        on_disk: u64,
    },
    /// Permission denied.
    #[error("permission denied at {path:?}")]
    PermissionDenied {
        /// Path that was denied.
        path: PathBuf,
    },
    /// Adapter-specific failure that does not fit the above.
    #[error("storage backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl StorageError {
    /// Build an `Io` error from a path + `io::Error`.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io { path: Some(path.into()), source }
    }

    /// Build an `Io` error without a path.
    pub(crate) fn io_bare(source: io::Error) -> Self {
        Self::Io { path: None, source }
    }
}
