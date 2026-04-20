//! `Repository<W>` port.
//!
//! `Repository` is the single writer. Direct writes to the underlying
//! log files are blocked by `knotch-linter`.
//!
//! Async trait methods use native Rust 2024 `async fn in trait` with
//! explicit `+ Send` bounds on returned futures. Adapters implement
//! directly; `DynRepository` (added later) wraps via pinned futures
//! for dyn-compatible usage.

use std::{future::Future, pin::Pin, sync::Arc};

use futures::Stream;

use crate::{
    error::RepositoryError,
    event::{AppendMode, AppendReport, Proposal, SubscribeEvent, SubscribeMode},
    id::UnitId,
    log::Log,
    workflow::WorkflowKind,
};

/// Pinned boxed stream alias used by Repository's subscribe-shaped
/// returns. Keeps the signature legible.
pub type PinStream<T> = Pin<Box<dyn Stream<Item = T> + Send + 'static>>;

/// Errors produced by [`ResumeCache`] accessors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// The stored value could not be deserialized into the requested type.
    #[error("cache entry {key:?} is not of the requested type")]
    TypeMismatch {
        /// Offending cache key.
        key: String,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
    /// The provided value could not be serialized.
    #[error("cannot serialize cache entry {key:?}")]
    Serialize {
        /// Key the caller tried to write.
        key: String,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
}

/// Opaque resume-cache payload — adapters pick their own on-disk
/// layout. The kernel hands out a `&mut ResumeCache` inside the
/// `Repository::with_cache` transaction; callers `get`/`set` typed
/// values.
#[derive(Debug, Default, Clone)]
pub struct ResumeCache {
    payload: serde_json::Map<String, serde_json::Value>,
}

impl ResumeCache {
    /// Construct an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a value, deserialized into the target type.
    ///
    /// Returns `Ok(None)` when the key is absent. Returns
    /// `Err(CacheError::TypeMismatch)` when the stored JSON cannot be
    /// deserialized into `T`.
    ///
    /// # Errors
    /// See [`CacheError`].
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>, CacheError> {
        match self.payload.get(key) {
            Some(v) => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|source| CacheError::TypeMismatch { key: key.to_owned(), source }),
            None => Ok(None),
        }
    }

    /// Set a value, replacing any previous entry.
    ///
    /// # Errors
    /// See [`CacheError`].
    pub fn set<T: serde::Serialize>(
        &mut self,
        key: impl Into<String>,
        value: &T,
    ) -> Result<(), CacheError> {
        let key = key.into();
        let value = serde_json::to_value(value)
            .map_err(|source| CacheError::Serialize { key: key.clone(), source })?;
        self.payload.insert(key, value);
        Ok(())
    }

    /// Remove a key. Returns `true` if the key was present.
    pub fn remove(&mut self, key: &str) -> bool {
        self.payload.remove(key).is_some()
    }

    /// Read-only view of the underlying map (used by adapters on save).
    #[must_use]
    pub fn as_map(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.payload
    }
}

/// Construct a cache from an existing map (used by adapters on load).
impl From<serde_json::Map<String, serde_json::Value>> for ResumeCache {
    fn from(payload: serde_json::Map<String, serde_json::Value>) -> Self {
        Self { payload }
    }
}

/// `Repository` port — single writer, async.
///
/// This is the multi-threaded version (every returned future is
/// `Send`). A `LocalRepository` variant without the Send bound is
/// generated later through `knotch-derive`; for now the library is
/// Send-only.
pub trait Repository<W: WorkflowKind>: Send + Sync + 'static {
    /// Borrow the workflow instance this repository was built against.
    /// Callers (hooks, skills, CLI) consult it for `required_phases` /
    /// `is_terminal_status` / `min_rationale_chars` / `parse_*` etc.
    fn workflow(&self) -> &W;

    /// Append proposals according to the supplied batching policy.
    ///
    /// # Errors
    /// Any non-success precondition surfaces as
    /// `RepositoryError::Precondition`; storage failures as `Storage`;
    /// lock contention as `Lock`.
    fn append(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
    ) -> impl Future<Output = Result<AppendReport<W>, RepositoryError>> + Send;

    /// Load the current event log for a unit. A missing unit returns
    /// an empty log.
    ///
    /// # Errors
    /// Surfaces storage or corruption errors.
    fn load(
        &self,
        unit: &UnitId,
    ) -> impl Future<Output = Result<Arc<Log<W>>, RepositoryError>> + Send;

    /// Load a point-in-time snapshot of the unit's log — every event
    /// with `at <= cutoff` included, everything after dropped. Useful
    /// for audit queries ("what was the state at 2026-03-15?") and
    /// time-travel debugging.
    ///
    /// The default implementation calls [`Self::load`] and filters
    /// client-side. Backends that can seek natively (future Postgres
    /// / SQLite adapters) override for efficiency.
    ///
    /// # Errors
    /// Same taxonomy as [`Self::load`].
    fn load_until(
        &self,
        unit: &UnitId,
        cutoff: crate::time::Timestamp,
    ) -> impl Future<Output = Result<Arc<Log<W>>, RepositoryError>> + Send {
        async move {
            let full = self.load(unit).await?;
            let filtered: Vec<_> =
                full.events().iter().take_while(|evt| evt.at <= cutoff).cloned().collect();
            Ok(Arc::new(Log::from_events(unit.clone(), filtered)))
        }
    }

    /// Subscribe to the live event stream for a unit.
    ///
    /// `FileRepository` and `InMemoryRepository` currently never
    /// return `Err` — the `Result` wrapper exists so Phase 10
    /// (cross-process file-watch, SQLite-backed repositories,
    /// etc.) can surface initial-subscription failures without
    /// breaking the trait. Later delivery failures appear as
    /// `SubscribeEvent::Lagged` entries on the stream.
    ///
    /// # Errors
    /// `RepositoryError::Storage` if the adapter cannot establish
    /// the initial subscription.
    fn subscribe(
        &self,
        unit: &UnitId,
        mode: SubscribeMode,
    ) -> impl Future<Output = Result<PinStream<SubscribeEvent<W>>, RepositoryError>> + Send;

    /// Enumerate known units (adapter-paginated).
    fn list_units(&self) -> PinStream<Result<UnitId, RepositoryError>>;

    /// Atomically append proposals and mutate the resume cache.
    ///
    /// `mutate_cache` runs under the unit's lock, after preconditions
    /// succeed but before the cache is persisted. On success, both
    /// sides commit together; on panic the transaction aborts and
    /// nothing is persisted.
    ///
    /// # Errors
    /// Same taxonomy as `append`.
    fn with_cache(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
        mutate_cache: CacheMutator,
    ) -> impl Future<Output = Result<AppendReport<W>, RepositoryError>> + Send;
}

/// Type-erased synchronous cache mutator. Callers that want async
/// cache mutation spawn their work on the surrounding runtime and
/// pass a `|cache| { cache.set(...)?; }` closure.
pub type CacheMutator = Box<dyn FnOnce(&mut ResumeCache) + Send + 'static>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_cache_roundtrips_values() {
        let mut cache = ResumeCache::new();
        cache.set("head", &"abc123").expect("set");
        let head: Option<String> = cache.get("head").expect("get");
        assert_eq!(head.as_deref(), Some("abc123"));
    }

    #[test]
    fn resume_cache_remove_reports_presence() {
        let mut cache = ResumeCache::new();
        cache.set("head", &"abc123").expect("set");
        assert!(cache.remove("head"));
        assert!(!cache.remove("head"));
    }
}
