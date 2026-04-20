//! Cross-platform advisory file locks for knotch.
//!
//! Locks are per-unit, advisory, and carry metadata sufficient to
//! reclaim a stale lock when the prior holder has died. Implementation
//! relies on `fs4` (cross-platform advisory locks) and `rustix`
//! (safe PID liveness probes). No `unsafe` lives in this crate —
//! the workspace-wide `#![forbid(unsafe_code)]` is honored.

pub mod metadata;

mod error;
mod file_lock;

pub use self::{
    error::LockError,
    file_lock::{FileLock, LockGuard},
    metadata::{LockMetadata, LockOwner},
};

use std::{future::Future, time::Duration};

use knotch_kernel::UnitId;

/// Port for per-unit advisory locks. Implementations provide a lease
/// + stale-reclaim semantic so orphaned locks cannot block forever.
pub trait Lock: Send + Sync + 'static {
    /// Acquire the lock for `unit`. Blocks up to `timeout` for the
    /// lock to become available; returns a `LockGuard` that releases
    /// on drop.
    ///
    /// A stale lock (prior holder's PID is not alive OR acquired_at +
    /// lease < now) is reclaimed transparently; the caller observes
    /// `Reclaimed` in the returned guard so it can emit a
    /// `ReconcileFailed { class: StaleLockReclaimed }` event.
    ///
    /// # Errors
    /// Returns a `LockError` on I/O failure, timeout, or refusal to
    /// reclaim.
    fn acquire(
        &self,
        unit: &UnitId,
        timeout: Duration,
        lease: Duration,
    ) -> impl Future<Output = Result<LockGuard, LockError>> + Send;
}
