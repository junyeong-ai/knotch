//! Structured rationale — the free-text "why" attached to gate
//! decisions, forced status transitions, and supersede events.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Minimum rationale length enforced by the default constructor.
/// Workflows can tighten this via `WorkflowKind::min_rationale_chars`
/// if they need a stricter floor.
pub const DEFAULT_MIN_RATIONALE_CHARS: usize = 8;

/// Maximum rationale length (UTF-8 bytes). Anything longer should be
/// a linked document, not an inline blob.
pub const MAX_RATIONALE_CHARS: usize = 8_192;

/// Structured rationale.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rationale(CompactString);

impl Rationale {
    /// Construct a `Rationale`, enforcing the default length bounds.
    ///
    /// # Errors
    /// Returns [`RationaleError::TooShort`] or [`RationaleError::TooLong`]
    /// when the supplied text fails the length invariants.
    pub fn new(text: impl Into<CompactString>) -> Result<Self, RationaleError> {
        Self::with_min(text, DEFAULT_MIN_RATIONALE_CHARS)
    }

    /// Construct with a caller-supplied minimum length (e.g. from a
    /// workflow's stricter policy).
    ///
    /// # Errors
    /// Same taxonomy as [`Self::new`].
    pub fn with_min(
        text: impl Into<CompactString>,
        min_chars: usize,
    ) -> Result<Self, RationaleError> {
        let text = text.into();
        let len = text.chars().count();
        if len < min_chars {
            return Err(RationaleError::TooShort { min: min_chars, actual: len });
        }
        if len > MAX_RATIONALE_CHARS {
            return Err(RationaleError::TooLong { max: MAX_RATIONALE_CHARS, actual: len });
        }
        Ok(Self(text))
    }

    /// Return the rationale text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Number of Unicode scalar values in the rationale.
    #[must_use]
    pub fn char_len(&self) -> usize {
        self.0.chars().count()
    }
}

/// Rationale-validation error taxonomy.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum RationaleError {
    /// Text shorter than the minimum-length invariant.
    #[error("rationale shorter than {min} chars (got {actual})")]
    TooShort {
        /// Minimum length.
        min: usize,
        /// Actual length.
        actual: usize,
    },
    /// Text longer than the maximum-length invariant.
    #[error("rationale longer than {max} chars (got {actual})")]
    TooLong {
        /// Maximum length.
        max: usize,
        /// Actual length.
        actual: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_rationale() {
        let err = Rationale::new("nope").unwrap_err();
        assert!(matches!(err, RationaleError::TooShort { .. }));
    }

    #[test]
    fn accepts_minimum_length() {
        let r = Rationale::new("exactly8").expect("should accept 8 chars");
        assert_eq!(r.char_len(), 8);
    }

    #[test]
    fn rejects_long_rationale() {
        let text = "x".repeat(MAX_RATIONALE_CHARS + 1);
        let err = Rationale::new(text).unwrap_err();
        assert!(matches!(err, RationaleError::TooLong { .. }));
    }

    #[test]
    fn counts_unicode_scalar_values_not_bytes() {
        // Four emoji = 4 chars but many bytes; with DEFAULT_MIN_RATIONALE_CHARS = 8
        // this should fail on char count.
        let err = Rationale::new("👍👍👍👍").unwrap_err();
        assert!(matches!(err, RationaleError::TooShort { actual: 4, .. }));
    }
}
