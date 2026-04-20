//! Immutable log snapshot type.
//!
//! A `Log<W>` is an append-only, ordered sequence of `Event<W>` plus
//! the `UnitId` that owns it. Repositories hand out `Arc<Log<W>>` so
//! projections can walk events without blocking the writer.

use std::sync::Arc;

use crate::{event::Event, id::UnitId, workflow::WorkflowKind};

/// Log-construction error.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum LogError {
    /// The event sequence violates the monotonic-timestamp invariant.
    #[error(
        "events are not monotonically ordered: events[{earlier_index}].at = {earlier} \
         > events[{later_index}].at = {later}"
    )]
    NonMonotonic {
        /// Index of the earlier-in-vec event that has a later
        /// timestamp than the following event.
        earlier_index: usize,
        /// Index of the following event.
        later_index: usize,
        /// Earlier-in-vec timestamp.
        earlier: jiff::Timestamp,
        /// Later-in-vec timestamp.
        later: jiff::Timestamp,
    },
}

/// Immutable, shared snapshot of a unit's event log.
#[derive(Debug)]
pub struct Log<W: WorkflowKind> {
    unit: UnitId,
    events: Arc<[Event<W>]>,
}

impl<W: WorkflowKind> Log<W> {
    /// Construct a log from an existing event sequence **without
    /// re-validating invariants**.
    ///
    /// This constructor is intended for Repository adapters that have
    /// already validated the sequence (either by building it themselves
    /// or by loading it from storage where the Repository wrote it).
    /// External callers should prefer [`Log::try_from_events`].
    ///
    /// In debug builds a `debug_assert!` verifies the monotonic-`at`
    /// invariant; release builds trust the caller.
    #[doc(hidden)]
    #[must_use]
    pub fn from_events(unit: UnitId, events: Vec<Event<W>>) -> Self {
        debug_assert!(
            events.windows(2).all(|w| w[0].at <= w[1].at),
            "Log::from_events: events are not monotonically ordered — use try_from_events",
        );
        Self { unit, events: Arc::from(events) }
    }

    /// Validated constructor — returns [`LogError::NonMonotonic`] if
    /// any adjacent pair of events is out of order. Suitable for
    /// external consumers (tests, CLI tools) that want to build a
    /// `Log` from hand-crafted events.
    ///
    /// # Errors
    /// See [`LogError`].
    pub fn try_from_events(unit: UnitId, events: Vec<Event<W>>) -> Result<Self, LogError> {
        for (i, pair) in events.windows(2).enumerate() {
            if pair[0].at > pair[1].at {
                return Err(LogError::NonMonotonic {
                    earlier_index: i,
                    later_index: i + 1,
                    earlier: pair[0].at,
                    later: pair[1].at,
                });
            }
        }
        Ok(Self { unit, events: Arc::from(events) })
    }

    /// Empty log for a unit.
    #[must_use]
    pub fn empty(unit: UnitId) -> Self {
        Self { unit, events: Arc::from(Vec::new()) }
    }

    /// Owning unit id.
    #[must_use]
    pub fn unit(&self) -> &UnitId {
        &self.unit
    }

    /// Event slice in append order.
    #[must_use]
    pub fn events(&self) -> &[Event<W>] {
        &self.events
    }

    /// Number of events in the log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Is the log empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl<W: WorkflowKind> Clone for Log<W> {
    fn clone(&self) -> Self {
        Self { unit: self.unit.clone(), events: self.events.clone() }
    }
}
