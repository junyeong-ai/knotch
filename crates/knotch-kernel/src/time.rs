//! Time primitives used throughout knotch.
//!
//! Knotch uses [`jiff`] for correctness across DST and timezone
//! arithmetic. This module re-exports the narrow subset knotch
//! itself touches, so downstream crates depend on this adapter rather
//! than on `jiff` directly — isolating us from pre-1.0 churn.

pub use jiff::{SignedDuration, Span, Timestamp, Zoned};

/// A monotonic clock abstraction. Real implementations return
/// `jiff::Timestamp::now()`; tests substitute a controllable clock.
pub trait Clock: Send + Sync {
    /// Return the current instant as a `jiff::Timestamp`.
    fn now(&self) -> Timestamp;
}

/// Concrete `Clock` backed by `jiff::Timestamp::now`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}
