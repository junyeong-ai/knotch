//! Status FSM — the workflow-independent lifecycle state.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// A lifecycle-status identifier. Workflow-independent; knotch does
/// not enumerate them. Presets supply their own status vocabularies
/// (Draft/Planning/In Progress/In Review/Archived).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StatusId(CompactString);

impl StatusId {
    /// Wrap a status name.
    #[must_use]
    pub fn new(name: impl Into<CompactString>) -> Self {
        Self(name.into())
    }

    /// Return the underlying name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for StatusId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Gate decision value. Enumerated because the set is universal
/// across presets (approve / reject / revise / defer).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Decision {
    /// Gate is cleared.
    Approved,
    /// Gate rejects; escalate or abandon.
    Rejected,
    /// Gate asks for revisions before re-submission.
    NeedsRevision,
    /// Gate deferred to a future pass.
    Deferred,
}

impl std::str::FromStr for Decision {
    type Err = String;

    /// Parse a human-supplied decision name (snake_case).
    ///
    /// # Errors
    /// Returns `Err` with a list of the accepted spellings when the
    /// input doesn't match any variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "needs_revision" => Ok(Self::NeedsRevision),
            "deferred" => Ok(Self::Deferred),
            other => Err(format!(
                "unknown decision `{other}` \
                 (expected `approved` | `rejected` | `needs_revision` | `deferred`)"
            )),
        }
    }
}
