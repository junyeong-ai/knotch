//! `SubagentStop` → append `EventBody::SubagentCompleted`.
//!
//! Every subagent termination funnels through `Repository::append`
//! with a canonical `EventBody::SubagentCompleted`. The subagent's
//! transcript path and last assistant message survive as event-
//! body fields, so `knotch-query` can filter by agent_id,
//! `project::subagents` can reconstruct the delegation roster, and
//! `cargo public-api` pins the wire shape — constitution §I (log
//! is the only truth).
//!
//! Called by the `record-subagent` CLI wrapper
//! (`crates/knotch-cli/src/cmd/hook/record_subagent.rs`).

use std::path::Path;

use compact_str::CompactString;
use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, UnitId, WorkflowKind, causation::AgentId,
    event::EventBody,
};
use serde::Serialize;

use crate::{error::HookError, output::HookOutput};

/// Append a `SubagentCompleted` event against the active unit.
///
/// The caller (the `record-subagent` CLI hook) already resolved the
/// active unit and the subagent fields from the Claude Code
/// `SubagentStop` stdin payload. This helper is generic over
/// `WorkflowKind` so the same append path works for any adopter
/// preset.
///
/// # Errors
///
/// Any `Repository::append` failure surfaces as
/// [`HookError::Repository`]. The caller typically ignores it (the
/// `record-subagent` hook is `SubagentStop`-triggered, which is a
/// post-event reporting signal — nothing to block).
pub async fn record<W, R>(
    repo: &R,
    unit: &UnitId,
    agent_id: impl Into<AgentId>,
    agent_type: Option<CompactString>,
    transcript_path: Option<&Path>,
    last_message: Option<CompactString>,
    causation: Causation,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let proposal = Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::SubagentCompleted {
            agent_id: agent_id.into(),
            agent_type,
            transcript_path: transcript_path.map(|p| CompactString::from(p.to_string_lossy())),
            last_message,
        },
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::BestEffort).await?;
    Ok(HookOutput::Continue)
}
