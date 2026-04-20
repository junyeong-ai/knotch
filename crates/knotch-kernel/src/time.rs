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

/// Stamp a monotonic timestamp — `max(clock.now(), last + 1ns)`.
///
/// Self-heals against wall-clock adjustments (NTP skew, operator
/// clock edits, VM pause/resume) without rejecting valid appends.
/// Whenever `last` is `Some`, the returned stamp is **strictly
/// greater** than `last`, so every event on a log carries a unique,
/// strictly increasing timestamp.
///
/// Deterministic on `(clock.now(), last)` per constitution §IX —
/// replay under a fixed clock produces identical stamps.
#[must_use]
pub fn stamp_monotonic(clock: &dyn Clock, last: Option<Timestamp>) -> Timestamp {
    let now = clock.now();
    match last {
        None => now,
        Some(prev) if now > prev => now,
        Some(prev) => prev.saturating_add(SignedDuration::from_nanos(1)).unwrap_or(prev),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedClock(Timestamp);

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            self.0
        }
    }

    fn ts(secs: i64) -> Timestamp {
        Timestamp::from_second(secs).expect("valid second")
    }

    #[test]
    fn first_stamp_is_clock_reading() {
        let clock = FixedClock(ts(1_000_000));
        assert_eq!(stamp_monotonic(&clock, None), ts(1_000_000));
    }

    #[test]
    fn forward_clock_passes_through() {
        let clock = FixedClock(ts(1_001));
        assert_eq!(stamp_monotonic(&clock, Some(ts(1_000))), ts(1_001));
    }

    #[test]
    fn equal_clock_bumps_by_one_nanosecond() {
        let t = ts(1_000);
        let clock = FixedClock(t);
        let at = stamp_monotonic(&clock, Some(t));
        assert!(at > t);
    }

    #[test]
    fn backward_clock_bumps_past_last() {
        let clock = FixedClock(ts(999));
        let last = ts(1_000);
        let at = stamp_monotonic(&clock, Some(last));
        assert!(at > last);
    }

    #[test]
    fn repeated_backward_clock_keeps_advancing() {
        let clock = FixedClock(ts(999));
        let first = stamp_monotonic(&clock, Some(ts(1_000)));
        let second = stamp_monotonic(&clock, Some(first));
        let third = stamp_monotonic(&clock, Some(second));
        assert!(third > second);
        assert!(second > first);
    }
}
