//! Scope — the workflow-scope selector, picked at unit creation.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Scope of a newly-created unit. Presets interpret scopes to choose
/// which phases are required (e.g. `Tiny` may skip REVIEW).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Scope {
    /// Minimal scope — typically skips one or more phases.
    Tiny,
    /// Default scope; all required phases must run.
    Standard,
    /// Large multi-story scope.
    Epic,
    /// Custom preset-defined scope name.
    Custom(CompactString),
}

impl Scope {
    /// Scope as a short machine-readable tag.
    #[must_use]
    pub fn tag(&self) -> &str {
        match self {
            Self::Tiny => "tiny",
            Self::Standard => "standard",
            Self::Epic => "epic",
            Self::Custom(s) => s.as_str(),
        }
    }
}
