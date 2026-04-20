//! Query error taxonomy.

use knotch_kernel::RepositoryError;

/// Errors returned by `QueryBuilder::execute`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// The underlying repository reported an error during
    /// `list_units` or `load`.
    #[error("repository failure")]
    Repository(#[source] RepositoryError),
}
