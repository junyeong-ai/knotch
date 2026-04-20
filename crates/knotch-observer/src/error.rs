//! Observer error taxonomy.

use compact_str::CompactString;

/// Errors surfaced by an `Observer::observe` call.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ObserverError {
    /// Observer exceeded its per-call deadline and was cancelled.
    #[error("observer {name} cancelled after {elapsed_ms} ms")]
    Cancelled {
        /// Observer name.
        name: CompactString,
        /// How long we ran before cancellation.
        elapsed_ms: u64,
    },
    /// Observer exceeded its per-call proposal budget.
    #[error("observer {name} exceeded budget of {limit} proposals")]
    BudgetExceeded {
        /// Observer name.
        name: CompactString,
        /// The budget that was exceeded.
        limit: usize,
    },
    /// VCS backend failure.
    #[error("vcs backend failure")]
    Vcs(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    /// Observer-specific failure that doesn't fit the above.
    #[error("observer backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}
