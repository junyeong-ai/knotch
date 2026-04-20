//! AI-agent harness integration for knotch.
//!
//! `knotch-agent` is the bridge between Claude Code (or any
//! analogous harness) hook/skill lifecycle and the knotch event
//! ledger. Every function is generic over `W: WorkflowKind`, so
//! the reference consumer (`knotch-cli`) and third-party consumers
//! share a single implementation.
//!
//! Responsibilities:
//! - parse hook stdin JSON into typed [`HookInput`]
//! - resolve the active unit from `.knotch/active.toml`
//! - translate tool calls (git commit, revert, destructive ops) and session events into
//!   knotch [`Proposal`](knotch_kernel::Proposal)s
//! - surface decisions as [`HookOutput`] (exit-0 continue, exit-0 context injection,
//!   exit-2 block)
//! - queue best-effort failures for the reconciler
//! - log orphan (pre-initialization) invocations without failing
//!
//! Binary entry points (`knotch-cli` and any third-party harness)
//! wrap these functions — adapter-specific logic is not permitted
//! here.

#![doc(test(attr(deny(warnings))))]

pub mod active;
pub mod atomic;
pub mod causation;
pub mod commit;
pub mod context;
pub mod error;
pub mod guard;
pub mod input;
pub mod model;
pub mod orphan;
pub mod output;
pub mod queue;
pub mod session;
pub mod session_end;
pub mod subagent;
pub mod tool_call;

pub use self::{
    active::{ActiveUnit, resolve_active, write_active},
    causation::hook_causation,
    error::HookError,
    input::{HookEvent, HookInput},
    output::HookOutput,
};
