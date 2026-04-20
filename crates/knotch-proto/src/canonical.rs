//! RFC 8785 JSON Canonicalization Scheme (JCS).
//!
//! Knotch fingerprints hash the *canonical* byte representation of an event's
//! dedup tuple, guaranteeing deterministic fingerprints across platforms and
//! serde versions.

use serde::Serialize;

/// Canonicalize a serde-serializable value into its RFC 8785 byte form.
///
/// # Errors
/// Returns the underlying `serde_json::Error` if serialization fails.
pub fn canonicalize<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(value)
}
