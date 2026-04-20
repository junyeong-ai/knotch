//! Wire format, canonical JSON (RFC 8785), schema versioning, and migration
//! registry for knotch.
//!
//! Split from `knotch-kernel` so that crates concerned only with the
//! on-disk representation — storage adapters, schema migrators,
//! fingerprint-verification tooling — pull this crate alone. The
//! kernel then layers semantics on top of the wire format without
//! forcing low-level consumers to drag in the full engine.

#![cfg_attr(docsrs, feature(doc_cfg))]

/// Current wire-format schema version.
///
/// Bumped only for backwards-incompatible wire changes. Every event log
/// file begins with a header line carrying this version; mismatches are
/// handled by the `migration::Registry`.
pub const SCHEMA_VERSION: u32 = 1;

pub mod canonical;
pub mod header;
pub mod migration;

// Compile-time guarantee; not a runtime test.
const _: () = assert!(SCHEMA_VERSION > 0, "SCHEMA_VERSION must be > 0");
