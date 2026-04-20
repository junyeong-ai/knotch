//! Observation context handed to every `Observer::observe` call.
//!
//! The context deliberately carries *shared* state only (log, cache,
//! cancel, wall clock). VCS and filesystem handles live inside the
//! observer that needs them, not in the context — keeping the
//! context dyn-safe and free of trait-object futures.

use std::{path::Path, sync::Arc};

use jiff::Timestamp;
use knotch_kernel::{Log, UnitId, WorkflowKind, repository::ResumeCache};
use tokio_util::sync::CancellationToken;

/// Read-only filesystem view. Fakes implement this in tests; the
/// default `StdFsView` proxies to `std::fs`.
pub trait FsView: Send + Sync {
    /// Does the path exist?
    fn exists(&self, path: &Path) -> bool;

    /// Recursively list regular-file paths under `root`, deterministic
    /// order. Directory entries and non-UTF-8 filenames are skipped.
    fn list_files(&self, root: &Path) -> Vec<std::path::PathBuf>;
}

/// `FsView` implementation backed by `std::fs`.
///
/// Reconciler observers typically run on `spawn_blocking`, so the
/// synchronous I/O here does not block the async runtime.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFsView;

impl FsView for StdFsView {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn list_files(&self, root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(ty) if ty.is_dir() => walk(&path, out),
                    Ok(ty) if ty.is_file() => out.push(path),
                    _ => {}
                }
            }
        }
        walk(root, &mut out);
        out.sort();
        out
    }
}

/// Per-observer resource budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObserveBudget {
    /// Maximum number of proposals the observer may return.
    pub max_proposals: usize,
}

impl Default for ObserveBudget {
    fn default() -> Self {
        Self { max_proposals: 128 }
    }
}

/// Context supplied to every observer.
pub struct ObserveContext<'a, W: WorkflowKind> {
    /// Unit being observed.
    pub unit: &'a UnitId,
    /// Snapshot of the event log taken at reconciler entry.
    pub log: Arc<Log<W>>,
    /// VCS HEAD sha at snapshot time.
    pub head: &'a str,
    /// Read-only view of the resume-cache watermark.
    pub cache: &'a ResumeCache,
    /// Wall-clock snapshot at reconciler entry.
    pub taken_at: Timestamp,
    /// Cooperative cancellation token.
    pub cancel: &'a CancellationToken,
    /// Per-observer budget.
    pub budget: ObserveBudget,
}
