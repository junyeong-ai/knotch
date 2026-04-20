//! Log-file header sentinel.

use serde::{Deserialize, Serialize};

/// First line written in every knotch log file.
///
/// Pinned `kind: "__header__"` discriminator keeps the header unambiguous
/// vs event lines; `schema_version` gates migration; `workflow` names the
/// `WorkflowKind` that produced the log; `fingerprint_salt` prevents
/// cross-workflow collisions and is written in base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename = "__header__")]
pub struct Header {
    /// Wire schema version (`knotch_proto::SCHEMA_VERSION` at write time).
    pub schema_version: u32,
    /// `WorkflowKind::NAME` value.
    pub workflow: compact_str::CompactString,
    /// Base64 (standard) of `WorkflowKind::fingerprint_salt()`.
    pub fingerprint_salt: compact_str::CompactString,
}
