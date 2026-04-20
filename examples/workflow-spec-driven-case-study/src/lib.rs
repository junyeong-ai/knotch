//! Case study: a spec-driven lifecycle
//! (SPECIFY → DESIGN → IMPLEMENT → REVIEW → WRAPUP) with story
//! milestones and a G0-G6 checkpoint-gate ladder (G4 reserved).
//!
//! Demonstrates how to fork the canonical `knotch_workflow::Knotch`
//! workflow into a project-specific `WorkflowKind` impl. Use this
//! source as a starting template when your domain needs different
//! phases, gates, or milestone types than the canonical workflow
//! ships.
//!
//! ```no_run
//! use workflow_spec_driven_case_study::{SpecDriven, build_repository};
//! let repo = build_repository("./state");
//! # let _: knotch_storage::FileRepository<SpecDriven> = repo;
//! ```

use std::{borrow::Cow, path::PathBuf};

use knotch_derive::{MilestoneKind, PhaseKind};
use knotch_kernel::{GateKind, Scope, WorkflowKind};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

/// Spec-driven lifecycle phases, declared in canonical order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum SpecPhase {
    /// Capture the user story and acceptance criteria.
    Specify,
    /// Shape the solution (plan, constitution, analyze docs).
    Design,
    /// Actually ship — milestone events flow from this phase.
    Implement,
    /// Peer / LLM review; blockers surface as G5 gates.
    Review,
    /// Wrap-up, ADR capture, archive.
    Wrapup,
}

/// Story milestone — a single user-visible unit of work.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct StoryId(pub compact_str::CompactString);

/// G0–G6 gate ladder for spec-driven development checkpoints.
///
/// Ordering is structural: each variant's [`GateKind::prerequisites`]
/// declares the earlier gates that must be on the log first. The
/// kernel enforces the graph on every `GateRecorded` append.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecGate {
    /// Scope assessment at the start of SPECIFY.
    G0Scope,
    /// Clarification pass over `[NEEDS CLARIFICATION]` markers.
    G1Clarify,
    /// Plan / constitution violations surface here.
    G2Constitution,
    /// Analyze findings (CRITICAL severity).
    G3Analyze,
    /// Review blockers on the PR thread.
    G5Review,
    /// Drift against upstream main since plan time.
    G6Drift,
}

const SPEC_PREREQ_G1: &[SpecGate] = &[SpecGate::G0Scope];
const SPEC_PREREQ_G2: &[SpecGate] = &[SpecGate::G0Scope, SpecGate::G1Clarify];
const SPEC_PREREQ_G3: &[SpecGate] =
    &[SpecGate::G0Scope, SpecGate::G1Clarify, SpecGate::G2Constitution];
const SPEC_PREREQ_G5: &[SpecGate] =
    &[SpecGate::G0Scope, SpecGate::G1Clarify, SpecGate::G2Constitution, SpecGate::G3Analyze];
const SPEC_PREREQ_G6: &[SpecGate] = &[
    SpecGate::G0Scope,
    SpecGate::G1Clarify,
    SpecGate::G2Constitution,
    SpecGate::G3Analyze,
    SpecGate::G5Review,
];

impl GateKind for SpecGate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::G0Scope => "g0-scope",
            Self::G1Clarify => "g1-clarify",
            Self::G2Constitution => "g2-constitution",
            Self::G3Analyze => "g3-analyze",
            Self::G5Review => "g5-review",
            Self::G6Drift => "g6-drift",
        })
    }

    fn prerequisites(&self) -> Cow<'_, [Self]> {
        match self {
            Self::G0Scope => Cow::Borrowed(&[]),
            Self::G1Clarify => Cow::Borrowed(SPEC_PREREQ_G1),
            Self::G2Constitution => Cow::Borrowed(SPEC_PREREQ_G2),
            Self::G3Analyze => Cow::Borrowed(SPEC_PREREQ_G3),
            Self::G5Review => Cow::Borrowed(SPEC_PREREQ_G5),
            Self::G6Drift => Cow::Borrowed(SPEC_PREREQ_G6),
        }
    }
}

/// Marker type carrying the `WorkflowKind` impl.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpecDriven;

const PHASES_TINY: [SpecPhase; 4] =
    [SpecPhase::Specify, SpecPhase::Design, SpecPhase::Implement, SpecPhase::Wrapup];

const PHASES_STANDARD: [SpecPhase; 5] = [
    SpecPhase::Specify,
    SpecPhase::Design,
    SpecPhase::Implement,
    SpecPhase::Review,
    SpecPhase::Wrapup,
];

const SPEC_DRIVEN_STATUSES: &[&str] =
    &["in_progress", "in_review", "shipped", "archived", "abandoned", "superseded", "deprecated"];

impl WorkflowKind for SpecDriven {
    type Phase = SpecPhase;
    type Milestone = StoryId;
    type Gate = SpecGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("specdriven")
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

    /// Terminal statuses for the spec-driven lifecycle. Non-forced
    /// transitions into these require every required phase to be
    /// resolved first (Phase × Status cross-invariant).
    fn is_terminal_status(&self, status: &knotch_kernel::StatusId) -> bool {
        matches!(status.as_str(), "archived" | "abandoned" | "superseded" | "deprecated")
    }

    /// Canonical spec-driven status vocabulary. Non-terminal
    /// statuses precede terminal ones.
    fn known_statuses(&self) -> Vec<Cow<'_, str>> {
        SPEC_DRIVEN_STATUSES.iter().map(|s| Cow::Borrowed(*s)).collect()
    }
}

/// Build a file-backed `SpecDriven` repository rooted at `root`.
pub fn build_repository(root: impl Into<PathBuf>) -> FileRepository<SpecDriven> {
    FileRepository::new(root, SpecDriven)
}

/// Low-level event-construction helpers. Consumers typically drive
/// the higher-level workflow binaries; these primitives stay
/// available for programmatic use.
pub mod events {
    use knotch_kernel::{
        Causation, Proposal, Rationale, StatusId,
        event::{ArtifactList, CommitKind, CommitRef, EventBody},
    };

    use super::{SpecDriven, SpecGate, SpecPhase, StoryId};

    /// `UnitCreated` with the given scope.
    pub fn unit_created(causation: Causation, scope: knotch_kernel::Scope) -> Proposal<SpecDriven> {
        Proposal {
            causation,
            extension: (),
            body: EventBody::UnitCreated { scope },
            supersedes: None,
        }
    }

    /// `PhaseCompleted` for `phase` with the supplied artifact list.
    pub fn phase_completed(
        causation: Causation,
        phase: SpecPhase,
        artifacts: ArtifactList,
    ) -> Proposal<SpecDriven> {
        Proposal {
            causation,
            extension: (),
            body: EventBody::PhaseCompleted { phase, artifacts },
            supersedes: None,
        }
    }

    /// `MilestoneShipped` for `story` against `commit`.
    pub fn milestone_shipped(
        causation: Causation,
        story: StoryId,
        commit: CommitRef,
        commit_kind: CommitKind,
    ) -> Proposal<SpecDriven> {
        Proposal {
            causation,
            extension: (),
            body: EventBody::MilestoneShipped {
                milestone: story,
                commit,
                commit_kind,
                status: knotch_kernel::CommitStatus::Verified,
            },
            supersedes: None,
        }
    }

    /// `GateRecorded` with the supplied decision and rationale.
    ///
    /// Gate ordering (G0 → G1 → G2 → G3 → G5 → G6) is enforced by
    /// the kernel at append time via
    /// [`SpecGate::prerequisites`](super::SpecGate::prerequisites);
    /// out-of-order proposals fail with
    /// `PreconditionError::GateOutOfOrder`.
    pub fn gate_recorded(
        causation: Causation,
        gate: SpecGate,
        decision: knotch_kernel::Decision,
        rationale: Rationale,
    ) -> Proposal<SpecDriven> {
        Proposal {
            causation,
            extension: (),
            body: EventBody::GateRecorded { gate, decision, rationale },
            supersedes: None,
        }
    }

    /// `StatusTransitioned` to the supplied target.
    pub fn status_transitioned(
        causation: Causation,
        target: StatusId,
        forced: bool,
        rationale: Option<Rationale>,
    ) -> Proposal<SpecDriven> {
        Proposal {
            causation,
            extension: (),
            body: EventBody::StatusTransitioned { target, forced, rationale },
            supersedes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use knotch_kernel::PhaseKind as _;

    use super::*;

    #[test]
    fn standard_scope_runs_every_phase() {
        let phases = SpecDriven.required_phases(&Scope::Standard);
        assert_eq!(phases.len(), 5);
        assert_eq!(phases[0], SpecPhase::Specify);
        assert_eq!(phases[4], SpecPhase::Wrapup);
    }

    #[test]
    fn tiny_scope_skips_review() {
        let phases = SpecDriven.required_phases(&Scope::Tiny);
        assert_eq!(phases.len(), 4);
        assert!(!phases.contains(&SpecPhase::Review));
    }

    #[test]
    fn phase_ids_match_canonical_kebab_form() {
        assert_eq!(SpecPhase::Specify.id(), "specify");
        assert_eq!(SpecPhase::Design.id(), "design");
        assert_eq!(SpecPhase::Wrapup.id(), "wrapup");
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SpecDriven.schema_version(), 1);
    }
}
