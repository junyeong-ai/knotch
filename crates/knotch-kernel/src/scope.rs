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

    /// Build a `Scope` from its tag form. Built-in variants (`tiny`
    /// / `standard` / `epic`) round-trip through their named
    /// variants; every other tag becomes `Scope::Custom(tag)`.
    ///
    /// Symmetric with [`Self::tag`]:
    /// `Scope::from_tag(s).tag() == s` for every `s`.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "tiny" => Self::Tiny,
            "standard" => Self::Standard,
            "epic" => Self::Epic,
            other => Self::Custom(CompactString::from(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_from_tag_roundtrip_is_symmetric() {
        for tag in ["tiny", "standard", "epic", "quick", "complex", "hotfix", "experiment"] {
            assert_eq!(Scope::from_tag(tag).tag(), tag, "tag `{tag}`");
        }
    }

    #[test]
    fn from_tag_maps_known_variants_to_named_arms() {
        assert!(matches!(Scope::from_tag("tiny"), Scope::Tiny));
        assert!(matches!(Scope::from_tag("standard"), Scope::Standard));
        assert!(matches!(Scope::from_tag("epic"), Scope::Epic));
    }

    #[test]
    fn from_tag_wraps_unknown_tags_in_custom() {
        match Scope::from_tag("quick") {
            Scope::Custom(s) => assert_eq!(s.as_str(), "quick"),
            other => panic!("expected Custom, got {other:?}"),
        }
    }
}
