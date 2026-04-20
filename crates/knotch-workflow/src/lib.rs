//! The canonical knotch workflow + supporting runtime types.
//!
//! This crate ships one opinionated workflow (the [`Knotch`] marker
//! type) plus the helpers every adopter-defined `WorkflowKind` needs:
//!
//! - [`Knotch`] — the canonical workflow: phases `Specify → Plan → Build → Review →
//!   Ship`, gates `G0..G4`, free-form [`TaskId`] milestones.
//! - [`PhaseOrdering`] — declarative ordering used by enum-backed phases and by
//!   runtime-defined [`DynamicPhase`] values.
//! - [`DynamicPhase`] / [`DynamicGate`] / [`DynamicMilestone`] — types that carry their
//!   name + spec at runtime; use when phases must be configurable rather than hard-coded.
//! - [`validate_ordering`] — acyclicity + uniqueness check, called by derived
//!   `WorkflowKind` impls.
//! - [`SkipPolicy`] — a reusable predicate describing which `SkipKind` values each phase
//!   accepts.

pub mod config;
pub mod dynamic;
pub mod knotch;
pub mod ordering;
pub mod skip;

pub use self::{
    config::{ConfigError, ConfigWorkflow, GateSpec, PhaseSpec, ScopedPhaseMap, WorkflowSpec},
    dynamic::{DynamicExtension, DynamicGate, DynamicMilestone, DynamicPhase},
    knotch::{Knotch, KnotchGate, KnotchPhase, TaskId, build_repository},
    ordering::{OrderingError, PhaseOrdering, validate_ordering},
    skip::SkipPolicy,
};
