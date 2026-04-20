//! Pure event-sourced workflow kernel.
//!
//! The kernel defines the type vocabulary and invariant contracts for
//! knotch: `WorkflowKind` and its associated types (`PhaseKind`,
//! `MilestoneKind`, `GateKind`, `ExtensionKind`), the `Event<W>` envelope
//! and `EventBody<W>` enum, `Causation` and its companions, the
//! `Repository<W>` trait, and a handful of free projections. The kernel
//! performs no I/O; adapters live in sibling crates
//! (`knotch-storage`, `-lock`, `-vcs`).
//!
//! See `/Users/mac/workspace/knotch/knotch-v1-final-plan.md` for the
//! full design.

#![doc(test(attr(deny(warnings))))]

pub mod causation;
pub mod error;
pub mod event;
pub mod fingerprint;
pub mod id;
pub mod log;
pub mod precondition;
pub mod project;
pub mod rationale;
pub mod repository;
pub mod scope;
pub mod status;
pub mod time;
pub mod workflow;

pub use self::{
    causation::{
        AgentId, Causation, Cost, Harness, ModelId, Person, Principal, SessionId, Source, TraceId,
        Trigger,
    },
    error::{PreconditionError, RepositoryError},
    event::{
        AppendMode, AppendReport, ArtifactList, CommitKind, CommitRef, CommitStatus, Event,
        EventBody, Proposal, ReconcileFailureKind, RetryAnchor, SkipKind, SubscribeEvent,
        SubscribeMode, ToolCallFailureReason,
    },
    fingerprint::{Fingerprint, fingerprint_event, fingerprint_proposal},
    id::{EventId, UNIT_ID_MAX_LEN, UnitId, UnitIdError},
    log::Log,
    rationale::Rationale,
    repository::{CacheError, CacheMutator, PinStream, Repository, ResumeCache},
    scope::Scope,
    status::{Decision, StatusId},
    workflow::{ExtensionKind, GateKind, MilestoneKind, PhaseKind, WorkflowKind},
};
