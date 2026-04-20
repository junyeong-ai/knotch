//! Storage adapters for knotch.
//!
//! A `Storage` adapter is responsible for the persistence of an event
//! log: appending a batch of serialized event lines, reading the log
//! back with corruption diagnostics, enumerating known units, and
//! storing the per-unit resume cache. Adapters own their own native
//! error type, boxed into `RepositoryError` at the kernel boundary.
//!
//! The sole shipped backend is `FileSystemStorage`, which writes one
//! event per line as JSONL with an atomic write-new-then-rename flow.
//! The `Storage` trait stays open for future adapters that bring
//! genuinely new capabilities (e.g. a Postgres-shaped multi-host
//! backend) — see `../CLAUDE.md` for the extension recipe. Embedded
//! KV / SQLite were considered and rejected: they add complexity
//! without measured benefit over JSONL for knotch's append-only,
//! single-unit-at-a-time access pattern.

pub mod atomic;
pub mod file_repository;
pub mod fs_storage;

mod error;
mod load_report;

pub use self::{
    error::StorageError,
    file_repository::FileRepository,
    fs_storage::FileSystemStorage,
    load_report::{CorruptionSpan, LoadReport},
};

use std::future::Future;

use futures::Stream;
use knotch_kernel::UnitId;

/// Port for event-log persistence.
///
/// Storage operates on byte-level line sequences; it does not know
/// about `Event<W>` — serialization is the Repository's job. This
/// keeps the adapter surface small and backend-agnostic (file system,
/// SQLite, redb, object storage).
pub trait Storage: Send + Sync + 'static {
    /// Read the raw JSONL contents for a unit's log.
    ///
    /// Returns an empty vector + empty `LoadReport` for a missing
    /// unit (not an error). A corrupted log returns the lines read
    /// so far plus diagnostics; the caller decides whether to surface
    /// a `RepositoryError::Corrupted` or continue with partial data.
    ///
    /// # Errors
    /// Returns a `StorageError` for genuine I/O failures only —
    /// corruption is reported in the `LoadReport`.
    fn load(
        &self,
        unit: &UnitId,
    ) -> impl Future<Output = Result<(Vec<String>, LoadReport), StorageError>> + Send;

    /// Append pre-serialized lines to a unit's log atomically.
    ///
    /// The adapter guarantees that after `append` returns `Ok`, the
    /// log file contains every prior line plus the newly appended
    /// lines, in order, or the file is unchanged.
    ///
    /// `expected_len` is the caller's view of the current line count
    /// (for optimistic concurrency). If the on-disk log is longer,
    /// the adapter returns `StorageError::LogMutated`.
    ///
    /// # Errors
    /// Returns a `StorageError` on I/O failure or optimistic-CAS
    /// mismatch.
    fn append(
        &self,
        unit: &UnitId,
        expected_len: u64,
        lines: Vec<String>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Enumerate known units. Adapters may page; the kernel does not
    /// assume any ordering unless documented.
    fn list_units(
        &self,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<UnitId, StorageError>> + Send + 'static>>;

    /// Read the resume-cache JSON for a unit, returning an empty map
    /// if absent.
    ///
    /// # Errors
    /// Returns a `StorageError` on I/O failure. A malformed JSON
    /// cache is rebuilt as empty (best-effort) — cache corruption
    /// is never fatal since the log remains the sole truth.
    fn read_cache(
        &self,
        unit: &UnitId,
    ) -> impl Future<
        Output = Result<serde_json::Map<String, serde_json::Value>, StorageError>,
    > + Send;

    /// Write the resume-cache JSON for a unit atomically.
    ///
    /// # Errors
    /// Returns a `StorageError` on I/O failure.
    fn write_cache(
        &self,
        unit: &UnitId,
        cache: serde_json::Map<String, serde_json::Value>,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}
