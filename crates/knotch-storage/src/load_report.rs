//! Corruption-aware load diagnostics.
//!
//! `Storage::load` returns the lines it could read plus a `LoadReport`.
//! A non-empty `corrupted` list means the log contained unreadable
//! lines; the kernel (via `Repository`) surfaces that as
//! `RepositoryError::Corrupted` unless the caller explicitly asked
//! for a partial-load.

use serde::{Deserialize, Serialize};

/// Diagnostics returned alongside `Storage::load`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadReport {
    /// 1-indexed corruption spans.
    pub corrupted: Vec<CorruptionSpan>,
    /// Total bytes read from the underlying source.
    pub bytes_read: u64,
}

/// A contiguous run of corrupted lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorruptionSpan {
    /// First corrupted line (1-indexed).
    pub start: u64,
    /// Last corrupted line (inclusive).
    pub end: u64,
    /// Human-readable reason.
    pub reason: String,
}

impl LoadReport {
    /// Is the report empty (no corruption)?
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.corrupted.is_empty()
    }

    /// First corrupted line, if any.
    #[must_use]
    pub fn first_corruption(&self) -> Option<&CorruptionSpan> {
        self.corrupted.first()
    }
}
