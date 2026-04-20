//! Runtime-defined phase / milestone / gate types.
//!
//! Enum-backed definitions (via `#[derive(PhaseKind)]` in
//! `knotch-derive`) are the common case; this module covers the
//! dynamic case where the workflow's phase set is known only at
//! runtime (e.g. loaded from config or declared by a plugin).

use std::{borrow::Cow, hash::Hash};

use compact_str::CompactString;
use knotch_kernel::{ExtensionKind, GateKind, MilestoneKind, PhaseKind, event::SkipKind};
use serde::{Deserialize, Serialize};

/// Runtime-configurable phase. Serialized as a bare string id so the
/// wire form matches typed enum-backed phases (e.g. `KnotchPhase`) —
/// typed and config-driven workflows can therefore produce
/// byte-identical logs for the same phase.
///
/// The accepts-skip list for a phase lives in the *workflow*
/// (`ConfigWorkflow`'s config), not on the phase value. Kernel
/// dispatch consults
/// [`WorkflowKind::accepts_skip_for`](knotch_kernel::WorkflowKind::accepts_skip_for).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DynamicPhase(pub CompactString);

impl DynamicPhase {
    /// Construct from any string-like id.
    pub fn new(id: impl Into<CompactString>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<CompactString> for DynamicPhase {
    fn from(s: CompactString) -> Self {
        Self(s)
    }
}

impl From<&str> for DynamicPhase {
    fn from(s: &str) -> Self {
        Self(CompactString::from(s))
    }
}

impl PhaseKind for DynamicPhase {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }

    fn is_skippable(&self, _reason: &SkipKind) -> bool {
        // Phase-level answer is always `false` for DynamicPhase —
        // `ConfigWorkflow::accepts_skip_for` consults the declared
        // `accepts_skips` list in `knotch.toml` to give the real
        // answer at the workflow level.
        false
    }
}

/// Runtime milestone identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DynamicMilestone(pub CompactString);

impl DynamicMilestone {
    /// Construct from any string-like id.
    pub fn new(id: impl Into<CompactString>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<CompactString> for DynamicMilestone {
    fn from(s: CompactString) -> Self {
        Self(s)
    }
}

impl From<&str> for DynamicMilestone {
    fn from(s: &str) -> Self {
        Self(CompactString::from(s))
    }
}

impl MilestoneKind for DynamicMilestone {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

/// Runtime gate identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DynamicGate(pub CompactString);

impl DynamicGate {
    /// Construct from any string-like id.
    pub fn new(id: impl Into<CompactString>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<CompactString> for DynamicGate {
    fn from(s: CompactString) -> Self {
        Self(s)
    }
}

impl From<&str> for DynamicGate {
    fn from(s: &str) -> Self {
        Self(CompactString::from(s))
    }
}

impl GateKind for DynamicGate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

/// Opaque dynamic extension — JSON-shaped typed payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DynamicExtension(pub serde_json::Value);

impl ExtensionKind for DynamicExtension {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_phase_serializes_as_bare_string() {
        let phase = DynamicPhase::from("specify");
        let j = serde_json::to_string(&phase).unwrap();
        assert_eq!(j, "\"specify\"");
        let back: DynamicPhase = serde_json::from_str(&j).unwrap();
        assert_eq!(back, phase);
    }

    #[test]
    fn dynamic_phase_equality_is_id_based() {
        assert_eq!(DynamicPhase::from("specify"), DynamicPhase::from("specify"));
        assert_ne!(DynamicPhase::from("specify"), DynamicPhase::from("plan"));
    }

    #[test]
    fn dynamic_milestone_round_trips() {
        let m = DynamicMilestone(CompactString::from("ship-signup"));
        let json = serde_json::to_string(&m).expect("ser");
        let back: DynamicMilestone = serde_json::from_str(&json).expect("de");
        assert_eq!(m, back);
    }
}
