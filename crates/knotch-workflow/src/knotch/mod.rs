//! The canonical knotch workflow.
//!
//! Five phases (`Specify → Plan → Build → Review → Ship`), five
//! checkpoint gates (`G0-G4`), a free-form [`TaskId`] milestone, and
//! a canonical status vocabulary. `Tiny` scope skips `Plan` and
//! `Review`.
//!
//! This is the one opinionated workflow knotch publishes. Adopters
//! whose shape differs write their own `WorkflowKind` impl — the
//! `examples/workflow-*-case-study/` directories ship reference
//! forks for the spec-driven, vibe, and ADR shapes.
//!
//! ```no_run
//! use knotch_workflow::Knotch;
//! use knotch_workflow::knotch::build_repository;
//! let repo = build_repository("./state");
//! # let _: knotch_storage::FileRepository<Knotch> = repo;
//! ```

use std::{borrow::Cow, path::PathBuf};

use compact_str::CompactString;
use knotch_derive::{MilestoneKind, PhaseKind};
use knotch_kernel::{GateKind, Scope, StatusId, WorkflowKind};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

pub mod events;

/// Phases of the canonical knotch workflow, in order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum KnotchPhase {
    /// Capture intent, audience, and acceptance criteria.
    Specify,
    /// Shape the solution (plan, design, constraints).
    Plan,
    /// Produce the change — milestone events flow from this phase.
    Build,
    /// Human / LLM review; blockers surface as `G3Review` gates.
    Review,
    /// Wrap-up, archive, capture follow-ups.
    Ship,
}

/// Checkpoint gates, ordered `G0..G4`.
///
/// Ordering is structural: each variant's [`GateKind::prerequisites`]
/// names the earlier gates that must be on the log before it can be
/// recorded. `EventBody::check_precondition` enforces the graph on
/// every append — there is no preflight-skipping path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnotchGate {
    /// Scope / fit decision at the start of `Specify`.
    G0Scope,
    /// Resolve `[NEEDS CLARIFICATION]` markers.
    G1Clarify,
    /// Plan / constitution / analyze blockers.
    G2Plan,
    /// Review blockers (PR thread, LLM pass).
    G3Review,
    /// Drift against main since plan time.
    G4Drift,
}

const KNOTCH_PREREQ_G1: &[KnotchGate] = &[KnotchGate::G0Scope];
const KNOTCH_PREREQ_G2: &[KnotchGate] = &[KnotchGate::G0Scope, KnotchGate::G1Clarify];
const KNOTCH_PREREQ_G3: &[KnotchGate] =
    &[KnotchGate::G0Scope, KnotchGate::G1Clarify, KnotchGate::G2Plan];
const KNOTCH_PREREQ_G4: &[KnotchGate] =
    &[KnotchGate::G0Scope, KnotchGate::G1Clarify, KnotchGate::G2Plan, KnotchGate::G3Review];

impl GateKind for KnotchGate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::G0Scope => "g0-scope",
            Self::G1Clarify => "g1-clarify",
            Self::G2Plan => "g2-plan",
            Self::G3Review => "g3-review",
            Self::G4Drift => "g4-drift",
        })
    }

    fn prerequisites(&self) -> Cow<'_, [Self]> {
        match self {
            Self::G0Scope => Cow::Borrowed(&[]),
            Self::G1Clarify => Cow::Borrowed(KNOTCH_PREREQ_G1),
            Self::G2Plan => Cow::Borrowed(KNOTCH_PREREQ_G2),
            Self::G3Review => Cow::Borrowed(KNOTCH_PREREQ_G3),
            Self::G4Drift => Cow::Borrowed(KNOTCH_PREREQ_G4),
        }
    }
}

/// Free-form milestone slug coined per unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct TaskId(pub CompactString);

/// The canonical knotch workflow marker type.
#[derive(Debug, Clone, Copy, Default)]
pub struct Knotch;

const PHASES_TINY: [KnotchPhase; 3] = [KnotchPhase::Specify, KnotchPhase::Build, KnotchPhase::Ship];

const PHASES_STANDARD: [KnotchPhase; 5] = [
    KnotchPhase::Specify,
    KnotchPhase::Plan,
    KnotchPhase::Build,
    KnotchPhase::Review,
    KnotchPhase::Ship,
];

const KNOTCH_STATUSES: &[&str] = &[
    "draft",
    "in_progress",
    "in_review",
    "shipped",
    "archived",
    "abandoned",
    "superseded",
    "deprecated",
];

impl WorkflowKind for Knotch {
    type Phase = KnotchPhase;
    type Milestone = TaskId;
    type Gate = KnotchGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("knotch")
    }
    fn schema_version(&self) -> u32 {
        1
    }

    fn required_phases(&self, scope: &Scope) -> Cow<'_, [Self::Phase]> {
        match scope {
            Scope::Tiny => Cow::Borrowed(&PHASES_TINY),
            _ => Cow::Borrowed(&PHASES_STANDARD),
        }
    }

    /// Terminal statuses for the knotch workflow. Non-forced
    /// transitions into these require every required phase to be
    /// resolved first (Phase × Status cross-invariant).
    fn is_terminal_status(&self, status: &StatusId) -> bool {
        matches!(status.as_str(), "archived" | "abandoned" | "superseded" | "deprecated")
    }

    /// Canonical status vocabulary. Non-terminal statuses precede
    /// terminal ones.
    fn known_statuses(&self) -> Vec<Cow<'_, str>> {
        KNOTCH_STATUSES.iter().map(|s| Cow::Borrowed(*s)).collect()
    }
}

/// Build a file-backed repository for the canonical knotch workflow.
#[must_use]
pub fn build_repository(root: impl Into<PathBuf>) -> FileRepository<Knotch> {
    FileRepository::new(root, Knotch)
}

#[cfg(test)]
mod tests {
    use knotch_kernel::PhaseKind as _;

    use super::*;

    #[test]
    fn standard_scope_runs_every_phase() {
        let phases = Knotch.required_phases(&Scope::Standard);
        assert_eq!(phases.len(), 5);
        assert_eq!(phases[0], KnotchPhase::Specify);
        assert_eq!(phases[4], KnotchPhase::Ship);
    }

    #[test]
    fn tiny_scope_skips_plan_and_review() {
        let phases = Knotch.required_phases(&Scope::Tiny);
        assert_eq!(phases.len(), 3);
        assert!(!phases.contains(&KnotchPhase::Plan));
        assert!(!phases.contains(&KnotchPhase::Review));
    }

    #[test]
    fn phase_id_matches_canonical_kebab_form() {
        assert_eq!(KnotchPhase::Specify.id(), "specify");
        assert_eq!(KnotchPhase::Plan.id(), "plan");
        assert_eq!(KnotchPhase::Build.id(), "build");
        assert_eq!(KnotchPhase::Review.id(), "review");
        assert_eq!(KnotchPhase::Ship.id(), "ship");
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(Knotch.schema_version(), 1);
    }

    #[test]
    fn known_statuses_includes_all_terminals() {
        let all = Knotch.known_statuses();
        for terminal in &["archived", "abandoned", "superseded", "deprecated"] {
            assert!(
                all.iter().any(|s| s.as_ref() == *terminal),
                "terminal status `{terminal}` missing from known_statuses",
            );
        }
    }

    #[test]
    fn terminal_detector_matches_vocabulary() {
        assert!(Knotch.is_terminal_status(&StatusId::new("archived")));
        assert!(Knotch.is_terminal_status(&StatusId::new("abandoned")));
        assert!(Knotch.is_terminal_status(&StatusId::new("superseded")));
        assert!(Knotch.is_terminal_status(&StatusId::new("deprecated")));
        assert!(!Knotch.is_terminal_status(&StatusId::new("in_progress")));
        assert!(!Knotch.is_terminal_status(&StatusId::new("shipped")));
    }
}
