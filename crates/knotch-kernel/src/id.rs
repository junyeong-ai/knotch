//! Identifier newtypes — `UnitId` and `EventId`.

use std::fmt;

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

/// Maximum byte length of a `UnitId` slug. Chosen to fit the common
/// `FEATURE-12345-my-slug` shape with room to spare while staying
/// comfortably under filesystem path-component limits on every
/// supported OS (typically 255 bytes, but the state-dir structure
/// joins slugs with prefixes so we reserve headroom).
pub const UNIT_ID_MAX_LEN: usize = 64;

/// Stable identifier for a workflow aggregate (a single unit of work:
/// a spec, an RFC, a changelog entry, etc.).
///
/// `UnitId` is an ASCII-only slug stored as a [`CompactString`] so the
/// common short-slug case is heap-free. The grammar is intentionally
/// conservative because slugs are used as **filesystem path
/// components** (under the state dir, the lock dir, the queue dir,
/// the subagent snapshot dir) and any looseness here becomes a
/// portability or security issue at the adapter boundary:
///
/// - Starts with an ASCII alphanumeric.
/// - Followed by ASCII alphanumeric, `-`, or `_`.
/// - Length 1..=[`UNIT_ID_MAX_LEN`].
///
/// Every other input — path separators, `..`, NUL, control chars,
/// whitespace, unicode that would NFC-normalize to a different path
/// component — is rejected by [`UnitId::try_new`]. The rule is the
/// same rule a POSIX-portable slug must satisfy, so a `UnitId` lands
/// as the same directory name on Linux, macOS, and Windows.
///
/// Deserialization applies the same validation — a log that somehow
/// contains an invalid slug fails loudly at load time rather than
/// silently carrying an unsafe identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct UnitId(CompactString);

impl UnitId {
    /// Construct a `UnitId` from a candidate slug, validating the
    /// grammar documented on the type. This is the only construction
    /// path exposed to external callers.
    ///
    /// # Errors
    /// Returns [`UnitIdError`] describing the first violation.
    pub fn try_new(slug: impl AsRef<str>) -> Result<Self, UnitIdError> {
        let s = slug.as_ref();
        validate_unit_slug(s)?;
        Ok(Self(CompactString::from(s)))
    }

    /// Return the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Internal adapter-only escape hatch — used when the caller has
    /// *already* validated the slug (e.g. enumerating directory names
    /// produced by a prior `try_new` call). Kept `#[doc(hidden)]` so
    /// the documented surface only shows the validating constructor.
    ///
    /// Do not call from tests, CLI code, or anywhere user input can
    /// flow in — use [`UnitId::try_new`] there.
    #[doc(hidden)]
    #[must_use]
    pub fn new_unchecked(slug: impl Into<CompactString>) -> Self {
        Self(slug.into())
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

impl<'de> Deserialize<'de> for UnitId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = CompactString::deserialize(d)?;
        validate_unit_slug(s.as_str()).map_err(serde::de::Error::custom)?;
        Ok(Self(s))
    }
}

/// Apply the [`UnitId`] grammar to a byte-for-byte slug and surface
/// the first violation. Pulled out of the constructor so `try_new`
/// and the custom `Deserialize` impl stay in lockstep.
fn validate_unit_slug(s: &str) -> Result<(), UnitIdError> {
    if s.is_empty() {
        return Err(UnitIdError::Empty);
    }
    if s.len() > UNIT_ID_MAX_LEN {
        return Err(UnitIdError::TooLong { actual: s.len(), max: UNIT_ID_MAX_LEN });
    }
    let mut chars = s.char_indices();
    let (_, first) = chars.next().expect("non-empty checked above");
    if !first.is_ascii_alphanumeric() {
        return Err(UnitIdError::InvalidStart { found: first });
    }
    for (position, c) in chars {
        let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if !ok {
            return Err(UnitIdError::InvalidChar { found: c, position });
        }
    }
    Ok(())
}

/// Reasons a slug is rejected by [`UnitId::try_new`]. Carries enough
/// context (the offending character plus its position) for operator
/// messages to say "character 'x' at position N" rather than a
/// generic "invalid slug".
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnitIdError {
    /// The slug was the empty string.
    #[error("unit id must not be empty")]
    Empty,
    /// Slug length exceeded [`UNIT_ID_MAX_LEN`].
    #[error("unit id exceeds max length: {actual} > {max}")]
    TooLong {
        /// Observed slug length in bytes.
        actual: usize,
        /// Configured maximum.
        max: usize,
    },
    /// First character was not ASCII alphanumeric. Common cause is a
    /// leading `-` / `_` / `.` / `/` slug — the last two in
    /// particular are path-traversal attempts.
    #[error("unit id must start with an ASCII alphanumeric, found {found:?}")]
    InvalidStart {
        /// The offending first character.
        found: char,
    },
    /// A character other than `[A-Za-z0-9_-]` was present. Covers
    /// path separators, control chars, whitespace, unicode, and `.`
    /// (which would open path-traversal via `..`).
    #[error("unit id character {found:?} at position {position} is not [A-Za-z0-9_-]")]
    InvalidChar {
        /// The offending character.
        found: char,
        /// Byte-offset position within the slug.
        position: usize,
    },
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

impl std::str::FromStr for EventId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(slug: &str) -> UnitId {
        UnitId::try_new(slug).unwrap_or_else(|e| panic!("expected `{slug}` to validate, got {e}"))
    }

    // ---- positive paths ----

    #[test]
    fn accepts_kebab_case() {
        let id = ok("signup-flow");
        assert_eq!(id.as_str(), "signup-flow");
    }

    #[test]
    fn accepts_alphanumeric_start_with_underscore_and_digit() {
        ok("A1");
        ok("v2_beta");
        ok("FEATURE-2026-04-20-sso-login");
    }

    #[test]
    fn roundtrips_through_json() {
        let id = UnitId::try_new("signup-flow").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let back: UnitId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    // ---- negative: structural rejects ----

    #[test]
    fn rejects_empty() {
        assert_eq!(UnitId::try_new(""), Err(UnitIdError::Empty));
    }

    #[test]
    fn rejects_too_long() {
        let over = "a".repeat(UNIT_ID_MAX_LEN + 1);
        assert!(matches!(
            UnitId::try_new(&over),
            Err(UnitIdError::TooLong { actual, max }) if actual == UNIT_ID_MAX_LEN + 1 && max == UNIT_ID_MAX_LEN
        ));
    }

    #[test]
    fn rejects_leading_non_alphanumeric() {
        for slug in ["-foo", "_foo", ".foo", "/foo", " foo"] {
            assert!(
                matches!(UnitId::try_new(slug), Err(UnitIdError::InvalidStart { .. })),
                "slug {slug:?} should reject as InvalidStart",
            );
        }
    }

    // ---- negative: path-traversal / filesystem-hazard inputs ----

    #[test]
    fn rejects_path_traversal() {
        for slug in ["..", "../etc/passwd", "foo/../bar", "foo/bar", r"foo\bar"] {
            let err = UnitId::try_new(slug).expect_err(&format!("slug {slug:?} must reject"));
            // Any of Empty / InvalidStart / InvalidChar is acceptable —
            // the guarantee is "does not validate" regardless of which
            // clause catches it first.
            assert!(
                matches!(err, UnitIdError::InvalidStart { .. } | UnitIdError::InvalidChar { .. }),
                "unexpected error variant {err:?} for slug {slug:?}",
            );
        }
    }

    #[test]
    fn rejects_nul_and_control_chars() {
        for slug in ["foo\0bar", "foo\nbar", "foo\tbar", "\x07beep"] {
            assert!(
                UnitId::try_new(slug).is_err(),
                "control-char slug {slug:?} should reject",
            );
        }
    }

    #[test]
    fn rejects_unicode() {
        // Any non-ASCII-alphanumeric is rejected. Covers NFC/NFD
        // lookalikes (Cyrillic 'а', full-width digits, zero-width
        // joiners) without a normalization pass.
        for slug in ["héllo", "hi\u{200b}there", "ｆｕｌｌｗｉｄｔｈ", "café"] {
            assert!(UnitId::try_new(slug).is_err(), "unicode slug {slug:?} should reject");
        }
    }

    #[test]
    fn rejects_whitespace_and_punctuation() {
        for slug in ["foo bar", "foo.bar", "foo:bar", "foo@bar", "foo*bar", "foo?bar"] {
            assert!(UnitId::try_new(slug).is_err(), "slug {slug:?} should reject");
        }
    }

    #[test]
    fn deserialize_rejects_invalid_slug() {
        // A log line that somehow contains an invalid UnitId must
        // fail at deserialization rather than silently carrying an
        // unsafe identifier into the repository layer. Which specific
        // grammar rule caught the slug (`InvalidStart` vs
        // `InvalidChar`) is not part of the wire contract — only that
        // "../etc/passwd" does not land as a UnitId.
        let json = r#""../etc/passwd""#;
        let err = serde_json::from_str::<UnitId>(json).expect_err("must reject on deserialize");
        let msg = err.to_string();
        assert!(
            msg.contains("must start with an ASCII alphanumeric")
                || msg.contains("not [A-Za-z0-9_-]"),
            "unexpected error message: {msg}",
        );
    }

    #[test]
    fn new_unchecked_is_doc_hidden_but_functional() {
        // Escape hatch for adapters that enumerate directories they
        // themselves populated via `try_new` — does not re-validate.
        let id = UnitId::new_unchecked("pre-validated");
        assert_eq!(id.as_str(), "pre-validated");
    }

    // ---- EventId ----

    #[test]
    fn event_ids_are_v7_and_sortable() {
        let a = EventId::new_v7();
        let b = EventId::new_v7();
        assert_ne!(a, b);
        assert_eq!(a.into_uuid().get_version_num(), 7);
        assert_eq!(b.into_uuid().get_version_num(), 7);
    }
}
