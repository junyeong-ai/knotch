//! Low-level event-construction helpers for the canonical knotch
//! workflow. Consumers typically drive the higher-level workflow
//! binaries (`knotch-cli`, `knotch-agent`) rather than calling these
//! directly; the primitives stay available for programmatic use and
//! for testing.

use knotch_kernel::{
    Causation, Proposal, Rationale, StatusId,
    event::{ArtifactList, CommitKind, CommitRef, EventBody},
};

use super::{Knotch, KnotchGate, KnotchPhase, TaskId};

/// `UnitCreated` with the given scope.
#[must_use]
pub fn unit_created(causation: Causation, scope: knotch_kernel::Scope) -> Proposal<Knotch> {
    Proposal {
        causation,
        extension: (),
        body: EventBody::UnitCreated { scope },
        supersedes: None,
    }
}

/// `PhaseCompleted` for `phase` with the supplied artifact list.
#[must_use]
pub fn phase_completed(
    causation: Causation,
    phase: KnotchPhase,
    artifacts: ArtifactList,
) -> Proposal<Knotch> {
    Proposal {
        causation,
        extension: (),
        body: EventBody::PhaseCompleted { phase, artifacts },
        supersedes: None,
    }
}

/// `MilestoneShipped` for `task` against `commit`.
#[must_use]
pub fn milestone_shipped(
    causation: Causation,
    task: TaskId,
    commit: CommitRef,
    commit_kind: CommitKind,
) -> Proposal<Knotch> {
    Proposal {
        causation,
        extension: (),
        body: EventBody::MilestoneShipped {
            milestone: task,
            commit,
            commit_kind,
            status: knotch_kernel::CommitStatus::Verified,
        },
        supersedes: None,
    }
}

/// `GateRecorded` with the supplied decision and rationale.
///
/// Gate ordering (G0 → G1 → G2 → G3 → G4) is enforced by the kernel
/// at append time via
/// [`KnotchGate::prerequisites`](super::KnotchGate::prerequisites);
/// out-of-order proposals fail with
/// `PreconditionError::GateOutOfOrder`. Callers do not need to
/// preflight the order themselves.
#[must_use]
pub fn gate_recorded(
    causation: Causation,
    gate: KnotchGate,
    decision: knotch_kernel::Decision,
    rationale: Rationale,
) -> Proposal<Knotch> {
    Proposal {
        causation,
        extension: (),
        body: EventBody::GateRecorded { gate, decision, rationale },
        supersedes: None,
    }
}

/// `StatusTransitioned` to the supplied target.
#[must_use]
pub fn status_transitioned(
    causation: Causation,
    target: StatusId,
    forced: bool,
    rationale: Option<Rationale>,
) -> Proposal<Knotch> {
    Proposal {
        causation,
        extension: (),
        body: EventBody::StatusTransitioned { target, forced, rationale },
        supersedes: None,
    }
}
