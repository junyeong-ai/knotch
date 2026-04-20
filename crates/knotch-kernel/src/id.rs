//! Identifier newtypes — `UnitId` and `EventId`.

use std::fmt;

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for a workflow aggregate (a single unit of work:
/// a spec, an RFC, a changelog entry, etc.).
///
/// `UnitId` is a user-chosen kebab-case slug, stored as a
/// [`CompactString`] so the common short-slug case is heap-free.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitId(CompactString);

impl UnitId {
    /// Construct a `UnitId` from anything convertible to a string.
    ///
    /// Knotch does not normalize the slug; callers pick their own
    /// convention. Empty strings are rejected at the `Repository`
    /// boundary, not here.
    #[must_use]
    pub fn new(slug: impl Into<CompactString>) -> Self {
        Self(slug.into())
    }

    /// Return the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for UnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for UnitId {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

/// Globally-unique event identifier.
///
/// Encoded as UUIDv7 (RFC 9562): 48-bit Unix-ms prefix + 74 bits of
/// entropy. Time-sortable, 128-bit, OTel-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    /// Generate a fresh v7 id using the process-global RNG.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    /// Unwrap the inner UUID.
    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.0
    }
}

/// Wrap a caller-supplied UUID as an `EventId`. Intended for
/// reproducible tests — production code should use
/// [`EventId::new_v7`].
impl From<Uuid> for EventId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_id_roundtrips_through_json() {
        let id = UnitId::new("signup-flow");
        let json = serde_json::to_string(&id).expect("serialize");
        let back: UnitId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn event_ids_are_v7_and_sortable() {
        let a = EventId::new_v7();
        let b = EventId::new_v7();
        assert_ne!(a, b);
        assert_eq!(a.into_uuid().get_version_num(), 7);
        assert_eq!(b.into_uuid().get_version_num(), 7);
    }
}
