//! Hook error taxonomy.
//!
//! Errors bubble up from the agent functions; the CLI entry point
//! decides the surface policy (retry/queue vs exit-2).

use thiserror::Error;

/// Unified error type across all `knotch-agent` functions.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HookError {
    /// Any `Repository::append` / `Repository::load` failure.
    #[error("repository: {0}")]
    Repository(#[from] knotch_kernel::RepositoryError),

    /// Filesystem I/O error while reading `.knotch/` state.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Hook stdin JSON parse failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// `.knotch/active.toml` or `.knotch/config.toml` parse failure.
    #[error("toml: {0}")]
    Toml(String),

    /// A blocking precondition surfaced. Maps to exit-2.
    #[error("blocked: {0}")]
    Blocked(String),

    /// `.knotch/` exists but no active unit is set — log orphan and
    /// exit 0 (non-blocking).
    #[error("no active unit (orphan)")]
    Orphan,

    /// Current directory is not a knotch project — silent no-op.
    #[error("not a knotch project")]
    NotAProject,
}
