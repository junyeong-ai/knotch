//! CQRS read-side query builder for knotch.
//!
//! `QueryBuilder<W>` composes predicates declaratively and executes
//! them against a `Repository<W>`. Implementation walks
//! `list_units()`, loads each log, and filters in memory. This is
//! intentionally simple — storage-native indexing (e.g. SQLite
//! views) is a future optimization and lives in adapter crates.

use std::borrow::Cow;

use futures::StreamExt as _;
use jiff::Timestamp;
use knotch_kernel::{
    Log, Repository, StatusId, UnitId, WorkflowKind,
    project::{current_phase, current_status, effective_events, shipped_milestones},
};

mod error;

pub use self::error::QueryError;

/// Fluent query over units.
pub struct QueryBuilder<W: WorkflowKind> {
    filters: Vec<Filter<W>>,
    limit: Option<usize>,
}

impl<W: WorkflowKind> Default for QueryBuilder<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: WorkflowKind> QueryBuilder<W> {
    /// Start an empty query.
    #[must_use]
    pub fn new() -> Self {
        Self { filters: Vec::new(), limit: None }
    }

    /// Match units whose current phase equals `phase`.
    #[must_use]
    pub fn where_phase(mut self, phase: W::Phase) -> Self {
        self.filters.push(Filter::Phase(phase));
        self
    }

    /// Match units that have shipped `milestone`.
    #[must_use]
    pub fn where_milestone_shipped(mut self, milestone: W::Milestone) -> Self {
        self.filters.push(Filter::MilestoneShipped(milestone));
        self
    }

    /// Match units whose current status equals `status`.
    #[must_use]
    pub fn where_status(mut self, status: StatusId) -> Self {
        self.filters.push(Filter::Status(status));
        self
    }

    /// Keep only units that have at least one effective event at or
    /// after `when`.
    #[must_use]
    pub fn since(mut self, when: Timestamp) -> Self {
        self.filters.push(Filter::Since(when));
        self
    }

    /// Keep only units that have at least one effective event at or
    /// before `when`.
    #[must_use]
    pub fn until(mut self, when: Timestamp) -> Self {
        self.filters.push(Filter::Until(when));
        self
    }

    /// Cap the number of returned units. Results are sorted by
    /// `UnitId` ascending before the limit is applied.
    #[must_use]
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Execute against `repo`.
    ///
    /// `workflow` is consulted for `current_phase` projection.
    ///
    /// # Errors
    /// Propagates `RepositoryError` wrapped as `QueryError::Repository`.
    pub async fn execute<R>(self, workflow: &W, repo: &R) -> Result<Vec<UnitId>, QueryError>
    where
        R: Repository<W>,
    {
        let mut units: Vec<UnitId> = {
            let mut stream = repo.list_units();
            let mut out = Vec::new();
            while let Some(next) = stream.next().await {
                out.push(next.map_err(QueryError::Repository)?);
            }
            out
        };
        units.sort_by(|a, b| a.as_str().cmp(b.as_str()));

        let mut matched = Vec::new();
        for unit in units {
            let log = repo.load(&unit).await.map_err(QueryError::Repository)?;
            if self.filters.iter().all(|f| f.matches(workflow, &log)) {
                matched.push(unit);
                if let Some(n) = self.limit {
                    if matched.len() >= n {
                        break;
                    }
                }
            }
        }
        Ok(matched)
    }
}

enum Filter<W: WorkflowKind> {
    Phase(W::Phase),
    MilestoneShipped(W::Milestone),
    Status(StatusId),
    Since(Timestamp),
    Until(Timestamp),
}

impl<W: WorkflowKind> Filter<W> {
    fn matches(&self, workflow: &W, log: &Log<W>) -> bool {
        match self {
            Filter::Phase(p) => current_phase(workflow, log).as_ref() == Some(p),
            Filter::MilestoneShipped(m) => {
                let shipped = shipped_milestones::<W>(log);
                shipped.iter().any(|s| milestone_id(s) == milestone_id(m))
            }
            Filter::Status(s) => current_status(log).as_ref() == Some(s),
            Filter::Since(ts) => effective_events(log).iter().any(|e| e.at >= *ts),
            Filter::Until(ts) => effective_events(log).iter().any(|e| e.at <= *ts),
        }
    }
}

fn milestone_id<M: knotch_kernel::MilestoneKind>(m: &M) -> Cow<'_, str> {
    m.id()
}
