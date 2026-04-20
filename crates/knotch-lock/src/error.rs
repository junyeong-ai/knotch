//! Lock error taxonomy.

use std::{io, path::PathBuf, time::Duration};

/// Errors surfaced by the `Lock` port.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LockError {
    /// Timed out waiting for the lock to become free.
    #[error("timed out after {waited:?} waiting for lock on unit {unit}")]
    Timeout {
        /// Unit the caller was trying to lock.
        unit: String,
        /// How long we waited before giving up.
        waited: Duration,
    },
    /// Refused to reclaim a lock whose prior holder is still alive.
    #[error(
        "lock on unit {unit} is held by pid {holder_pid} on {holder_host:?} — still alive"
    )]
    Contended {
        /// Unit the caller was trying to lock.
        unit: String,
        /// PID recorded in the lock metadata.
        holder_pid: u32,
        /// Hostname recorded in the lock metadata.
        holder_host: Option<String>,
    },
    /// Underlying I/O failure.
    #[error("lock I/O error at {path:?}")]
    Io {
        /// Path that failed, if known.
        path: Option<PathBuf>,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Metadata failed to parse.
    #[error("lock metadata at {path:?} is malformed")]
    MalformedMetadata {
        /// Path that failed.
        path: PathBuf,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
}

impl LockError {
    /// Build an `Io` error from a path + `io::Error`.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io { path: Some(path.into()), source }
    }
}
