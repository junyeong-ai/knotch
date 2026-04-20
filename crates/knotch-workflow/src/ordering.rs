//! Phase ordering: declarative ordered sequence of phase ids with
//! acyclicity + uniqueness validation.

use compact_str::CompactString;

/// Declarative ordering of phase ids. Used both by enum-backed
/// phases (consumed at compile time by the derive macro) and by
/// `DynamicPhase` instances (consumed at runtime via `Workflow::builder`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseOrdering {
    order: Vec<CompactString>,
}

impl PhaseOrdering {
    /// Build an ordering from an iterator of phase ids.
    ///
    /// # Errors
    /// Returns `OrderingError::Duplicate` if any id repeats;
    /// `OrderingError::Empty` if the iterator is empty.
    pub fn new<I, S>(ids: I) -> Result<Self, OrderingError>
    where
        I: IntoIterator<Item = S>,
        S: Into<CompactString>,
    {
        let order: Vec<CompactString> = ids.into_iter().map(Into::into).collect();
        validate_ordering(&order)?;
        Ok(Self { order })
    }

    /// Iterate phase ids in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.order.iter().map(CompactString::as_str)
    }

    /// Number of phases in this ordering.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Is the ordering empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Return the phase id that follows `id`, if any.
    #[must_use]
    pub fn next_after(&self, id: &str) -> Option<&str> {
        let pos = self.order.iter().position(|x| x == id)?;
        self.order.get(pos + 1).map(CompactString::as_str)
    }

    /// Check membership in O(n).
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.order.iter().any(|x| x == id)
    }
}

/// Acyclicity + uniqueness check for a flat phase sequence.
///
/// A flat `Vec<id>` is trivially acyclic; the validation here is
/// uniqueness (same id cannot appear twice) + non-emptiness. The
/// naming preserves the "acyclicity" intuition so the derive macro
/// can call this under a single name today and expand to graph
/// analysis tomorrow.
///
/// # Errors
/// Returns `OrderingError::Duplicate` or `OrderingError::Empty`.
pub fn validate_ordering(order: &[CompactString]) -> Result<(), OrderingError> {
    if order.is_empty() {
        return Err(OrderingError::Empty);
    }
    for (i, id) in order.iter().enumerate() {
        if id.is_empty() {
            return Err(OrderingError::EmptyId { index: i });
        }
        if order[..i].iter().any(|prior| prior == id) {
            return Err(OrderingError::Duplicate { id: id.clone() });
        }
    }
    Ok(())
}

/// Ordering validation failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderingError {
    /// Same phase id declared more than once.
    #[error("phase id {id:?} declared more than once")]
    Duplicate {
        /// The repeated id.
        id: CompactString,
    },
    /// An empty string was used as a phase id.
    #[error("phase id at position {index} is empty")]
    EmptyId {
        /// Position of the empty id.
        index: usize,
    },
    /// Ordering has no phases.
    #[error("phase ordering is empty")]
    Empty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty_unique_sequence() {
        let ordering = PhaseOrdering::new(["specify", "design", "implement"]).expect("build");
        assert_eq!(ordering.len(), 3);
        assert_eq!(ordering.next_after("specify"), Some("design"));
        assert_eq!(ordering.next_after("implement"), None);
        assert!(ordering.contains("design"));
    }

    #[test]
    fn rejects_duplicates() {
        let err = PhaseOrdering::new(["a", "b", "a"]).unwrap_err();
        assert_eq!(err, OrderingError::Duplicate { id: CompactString::from("a") });
    }

    #[test]
    fn rejects_empty_sequence() {
        let err = PhaseOrdering::new::<[&str; 0], _>([]).unwrap_err();
        assert_eq!(err, OrderingError::Empty);
    }

    #[test]
    fn rejects_empty_id() {
        let err = PhaseOrdering::new(["specify", ""]).unwrap_err();
        assert_eq!(err, OrderingError::EmptyId { index: 1 });
    }

    #[test]
    fn validate_helper_matches_public_constructor() {
        let seq: Vec<CompactString> = ["a", "b", "c"].iter().map(|s| (*s).into()).collect();
        assert!(validate_ordering(&seq).is_ok());
    }
}
