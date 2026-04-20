//! Content-addressed event fingerprint.
//!
//! `Fingerprint` is a 32-byte BLAKE3 hash of the RFC 8785 JCS
//! canonical form of a per-variant dedup tuple, prefixed with the
//! workflow-specific salt from [`WorkflowKind::fingerprint_salt`].
//! Two events with the same fingerprint are, by definition, the
//! same proposal — the Repository treats the second as a duplicate.
//!
//! Fingerprint derivation is **closed**: users cannot swap the
//! algorithm. Workflows that need distinct dedup semantics override
//! [`WorkflowKind::fingerprint_salt`] or define a distinct
//! `WorkflowKind`. See `.claude/rules/fingerprint.md` for the
//! rationale.
//!
//! [`WorkflowKind::fingerprint_salt`]: crate::workflow::WorkflowKind::fingerprint_salt

use std::fmt;

use serde::{Deserialize, Serialize};

/// A 256-bit content-addressed event fingerprint.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Return the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Compute a fingerprint from an arbitrary byte slice.
    ///
    /// The caller is responsible for producing canonical bytes —
    /// typically [`knotch_proto::canonical::canonicalize`] output
    /// prefixed with the workflow's salt.
    #[must_use]
    pub fn hash(canonical_bytes: &[u8]) -> Self {
        let hash = blake3::hash(canonical_bytes);
        Self(*hash.as_bytes())
    }

    /// Render the fingerprint as lowercase hex. Used in debug output
    /// and CLI rendering.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.0 {
            use std::fmt::Write as _;
            write!(&mut out, "{b:02x}").expect("write to String cannot fail");
        }
        out
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Truncate in debug so printouts remain readable.
        let hex = self.to_hex();
        write!(f, "Fingerprint({}…)", &hex[..16])
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl From<[u8; 32]> for Fingerprint {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Compute the canonical fingerprint of a [`Proposal`](crate::Proposal).
///
/// The dedup tuple is
/// `{ "workflow": W::NAME, "body": …, "supersedes": optional }` —
/// canonicalized via RFC 8785 JCS (byte-identical across platforms,
/// serde versions, and key-insertion orders), then BLAKE3-hashed with
/// [`WorkflowKind::fingerprint_salt`](crate::WorkflowKind::fingerprint_salt)
/// prefixed. Two workflows sharing the same storage root cannot
/// collide.
///
/// # Errors
/// Returns `serde_json::Error` if the proposal body fails to
/// serialize — typically a custom `Extension` type that itself
/// fails to serialize.
pub fn fingerprint_proposal<W>(
    workflow: &W,
    proposal: &crate::Proposal<W>,
) -> Result<Fingerprint, serde_json::Error>
where
    W: crate::WorkflowKind,
{
    fingerprint_parts::<W>(workflow, &proposal.body, proposal.supersedes)
}

/// Recompute an event's fingerprint from its stored body — used by
/// Repository adapters to detect duplicates on reload.
///
/// # Errors
/// Same taxonomy as [`fingerprint_proposal`].
pub fn fingerprint_event<W>(
    workflow: &W,
    event: &crate::Event<W>,
) -> Result<Fingerprint, serde_json::Error>
where
    W: crate::WorkflowKind,
{
    fingerprint_parts::<W>(workflow, &event.body, event.supersedes)
}

fn fingerprint_parts<W: crate::WorkflowKind>(
    workflow: &W,
    body: &crate::event::EventBody<W>,
    supersedes: Option<crate::EventId>,
) -> Result<Fingerprint, serde_json::Error> {
    let mut key = serde_json::Map::new();
    key.insert(
        "workflow".into(),
        serde_json::Value::String(workflow.name().into_owned()),
    );
    key.insert("body".into(), serde_json::to_value(body)?);
    if let Some(target) = supersedes {
        key.insert("supersedes".into(), serde_json::to_value(target)?);
    }
    // RFC 8785 JSON Canonicalization — keys sorted lexicographically,
    // numbers normalized, whitespace stripped.
    let canonical = serde_jcs::to_vec(&serde_json::Value::Object(key))?;
    let salt = workflow.fingerprint_salt();
    let mut salted = Vec::with_capacity(canonical.len() + salt.len());
    salted.extend_from_slice(&salt);
    salted.extend_from_slice(&canonical);
    Ok(Fingerprint::hash(&salted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let a = Fingerprint::hash(b"hello knotch");
        let b = Fingerprint::hash(b"hello knotch");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_differs_for_different_inputs() {
        let a = Fingerprint::hash(b"a");
        let b = Fingerprint::hash(b"b");
        assert_ne!(a, b);
    }

    #[test]
    fn hex_round_trips_through_length() {
        let fp = Fingerprint::hash(b"knotch");
        assert_eq!(fp.to_hex().len(), 64);
    }

    // --- JCS-canonicalization property tests -------------------------
    //
    // These lock in the fingerprint-determinism guarantee: the tuple
    // that gets hashed is byte-identical regardless of JSON-object key
    // order, numeric format, or whitespace.

    use serde_json::{Value, json};

    fn hash_object(obj: serde_json::Map<String, Value>) -> Fingerprint {
        let bytes = serde_jcs::to_vec(&Value::Object(obj)).expect("jcs");
        Fingerprint::hash(&bytes)
    }

    #[test]
    fn jcs_canonicalization_normalizes_key_order() {
        let mut a = serde_json::Map::new();
        a.insert("alpha".into(), json!(1));
        a.insert("beta".into(), json!(2));
        let mut b = serde_json::Map::new();
        b.insert("beta".into(), json!(2));
        b.insert("alpha".into(), json!(1));
        assert_eq!(hash_object(a), hash_object(b));
    }

    #[test]
    fn jcs_canonicalization_normalizes_numeric_form() {
        // serde_jcs lowers both integer forms to the same canonical
        // number representation.
        let mut a = serde_json::Map::new();
        a.insert("n".into(), json!(1));
        let mut b = serde_json::Map::new();
        b.insert("n".into(), json!(1_i64));
        assert_eq!(hash_object(a), hash_object(b));
    }

    #[test]
    fn jcs_whitespace_does_not_affect_hash() {
        // Canonical bytes are whitespace-free by construction.
        let canonical = serde_jcs::to_vec(&json!({"a": 1, "b": 2})).expect("jcs");
        assert!(!canonical.contains(&b' '));
        assert!(!canonical.contains(&b'\n'));
    }
}
