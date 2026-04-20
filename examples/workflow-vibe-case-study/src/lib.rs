//! AI-pair-programming preset — phases `Intent → Explore →
//! Implement → Verify`, tuned for agent-driven development.
//!
//! The preset ships:
//!
//! - [`Vibe`] — the `WorkflowKind` impl plus milestone/gate shapes. Milestones are
//!   free-form ids so agents can coin names.
//! - [`Session`] — a `Causation`-factory that tags every event with agent/model/session
//!   metadata, making cost and attribution first-class.
//! - [`total_usd`] / [`total_tokens`] — roll-ups over an effective log.
//! - [`summary_for_llm`] — LLM-friendly natural-language summary budget-capped to a
//!   target token count (approximated by chars).
//! - [`build_repository`] — one-liner file-backed repo.

use std::{borrow::Cow, path::PathBuf};

use compact_str::CompactString;
use jiff::Timestamp;
use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{
    Causation, Log, PhaseKind as _, Scope, WorkflowKind,
    causation::{AgentId, Cost, Harness, ModelId, Principal, SessionId, Source, Trigger},
    event::EventBody,
    project::{current_phase, current_status, effective_events, total_cost},
};
use knotch_storage::FileRepository;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Vibe-coding lifecycle phases.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum VibePhase {
    /// Human states intent / target outcome.
    Intent,
    /// Agent explores the codebase and proposes a plan.
    Explore,
    /// Agent writes code, tests, migrations.
    Implement,
    /// Agent + human confirm behavior & ship.
    Verify,
}

/// Milestone id — free-form short name coined per unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct TaskId(pub CompactString);

/// Gates at which the agent hands control back to the human.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum VibeGate {
    /// Intent is clear enough to start exploring.
    IntentClear,
    /// Hand the work to another agent / human.
    Handoff,
}

/// Workflow marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Vibe;

const PHASES: [VibePhase; 4] =
    [VibePhase::Intent, VibePhase::Explore, VibePhase::Implement, VibePhase::Verify];

const VIBE_STATUSES: &[&str] =
    &["in_progress", "in_review", "shipped", "archived", "abandoned", "handed_off"];

impl WorkflowKind for Vibe {
    type Phase = VibePhase;
    type Milestone = TaskId;
    type Gate = VibeGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("vibe")
    }
    fn schema_version(&self) -> u32 {
        1
    }

    fn required_phases(&self, _: &Scope) -> Cow<'_, [Self::Phase]> {
        Cow::Borrowed(&PHASES)
    }

    fn min_rationale_chars(&self) -> usize {
        // Vibe rationales are typically short agent self-reports.
        4
    }

    /// Terminal statuses for the vibe workflow.
    fn is_terminal_status(&self, status: &knotch_kernel::StatusId) -> bool {
        matches!(status.as_str(), "archived" | "abandoned" | "handed_off")
    }

    /// Canonical status vocabulary for the vibe workflow. Non-terminal
    /// statuses precede terminal ones.
    fn known_statuses(&self) -> Vec<Cow<'_, str>> {
        VIBE_STATUSES.iter().map(|s| Cow::Borrowed(*s)).collect()
    }
}

/// Session — the factory every agent-initiated event flows through.
///
/// A `Session` is cheap to clone; it carries the session id plus the
/// agent / model / harness identifiers. Consumers build a new
/// session once per conversation and call [`Session::causation`]
/// at each proposal site.
#[derive(Debug, Clone)]
pub struct Session {
    id: SessionId,
    agent_id: AgentId,
    model: ModelId,
    harness: Harness,
}

impl Session {
    /// Start a new session.
    pub fn new(
        agent: impl Into<CompactString>,
        model: impl Into<CompactString>,
        harness: impl Into<CompactString>,
    ) -> Self {
        Self {
            id: SessionId::new_v7(),
            agent_id: AgentId(agent.into()),
            model: ModelId(model.into()),
            harness: Harness(harness.into()),
        }
    }

    /// Session identifier.
    #[must_use]
    pub fn id(&self) -> SessionId {
        self.id.clone()
    }

    /// Build a `Causation` for the current session. Callers chain
    /// `with_cost` / `with_trace` / `with_parent_event` as needed.
    #[must_use]
    pub fn causation(&self, trigger: Trigger) -> Causation {
        Causation::new(
            Source::Agent,
            Principal::Agent {
                agent_id: self.agent_id.clone(),
                model: self.model.clone(),
                harness: self.harness.clone(),
            },
            trigger,
        )
        .with_session(self.id.clone())
    }

    /// Convenience — `Trigger::ToolInvocation` causation.
    #[must_use]
    pub fn tool(
        &self,
        tool: impl Into<CompactString>,
        call_id: impl Into<CompactString>,
    ) -> Causation {
        self.causation(Trigger::ToolInvocation { tool: tool.into(), call_id: call_id.into() })
    }
}

/// Build a file-backed `Vibe` repository rooted at `root`.
pub fn build_repository(root: impl Into<PathBuf>) -> FileRepository<Vibe> {
    FileRepository::new(root, Vibe)
}

/// Sum of every `Causation::cost.usd` entry on the effective log.
#[must_use]
pub fn total_usd(log: &Log<Vibe>) -> Option<Decimal> {
    total_cost(log).usd
}

/// Sum of input + output tokens across the effective log.
#[must_use]
pub fn total_tokens(log: &Log<Vibe>) -> (u64, u64) {
    let c = total_cost(log);
    (u64::from(c.tokens_in), u64::from(c.tokens_out))
}

/// Budgetted summarization of a unit's log for prompt injection.
#[derive(Debug, Clone)]
pub struct LlmSummary {
    /// Human-readable markdown body.
    pub body: String,
    /// Approximate token count (chars / 4).
    pub approx_tokens: usize,
}

/// Budget for `summary_for_llm` — approximates tokens via character
/// counts (≈4 chars per token).
#[derive(Debug, Clone, Copy)]
pub struct SummaryBudget {
    /// Maximum approximate tokens to produce.
    pub max_tokens: usize,
}

impl Default for SummaryBudget {
    fn default() -> Self {
        Self { max_tokens: 2_048 }
    }
}

/// Produce an LLM-friendly summary of a vibe-workflow log.
///
/// The summary renders current phase, status, shipped milestones,
/// aggregated cost, and the most recent events — trimmed to the
/// supplied [`SummaryBudget`].
#[must_use]
pub fn summary_for_llm(log: &Log<Vibe>, budget: SummaryBudget) -> LlmSummary {
    let max_chars = budget.max_tokens.saturating_mul(4);
    let mut body = String::with_capacity(max_chars.min(4_096));

    body.push_str("## knotch unit summary\n");
    if let Some(phase) = current_phase(&Vibe, log) {
        body.push_str(&format!("- current phase: **{}**\n", phase.id()));
    }
    if let Some(status) = current_status(log) {
        body.push_str(&format!("- current status: `{}`\n", status.as_str()));
    }
    let cost = total_cost(log);
    body.push_str(&format!(
        "- cost so far: {} USD · {} in / {} out tokens\n",
        cost.usd.map(|d| d.to_string()).unwrap_or_else(|| "?".into()),
        cost.tokens_in,
        cost.tokens_out,
    ));

    body.push_str("\n## recent events\n");
    let effective = effective_events(log);
    for evt in effective.iter().rev() {
        let line = format!("- {} · {} · {}\n", evt.at, event_tag(&evt.body), short_detail(evt),);
        if body.len() + line.len() > max_chars {
            body.push_str("- …\n");
            break;
        }
        body.push_str(&line);
    }

    let approx_tokens = body.chars().count() / 4;
    LlmSummary { body, approx_tokens }
}

fn event_tag(body: &EventBody<Vibe>) -> &'static str {
    // Delegate to `EventBody::kind_tag` — the single source of truth.
    body.kind_tag()
}

fn short_detail(evt: &knotch_kernel::Event<Vibe>) -> String {
    match &evt.body {
        EventBody::UnitCreated { scope } => format!("scope={}", scope.tag()),
        EventBody::PhaseCompleted { phase, .. } => format!("phase={}", phase.id()),
        EventBody::PhaseSkipped { phase, reason } => {
            format!("phase={} reason={reason:?}", phase.id())
        }
        EventBody::MilestoneShipped { milestone, commit, .. } => {
            format!("milestone={} commit={}", milestone.0, commit.as_str())
        }
        EventBody::MilestoneReverted { milestone, revert, .. } => {
            format!("milestone={} revert={}", milestone.0, revert.as_str())
        }
        EventBody::MilestoneVerified { milestone, commit } => {
            format!("milestone={} commit={}", milestone.0, commit.as_str())
        }
        EventBody::GateRecorded { gate, decision, .. } => {
            format!("gate={gate:?} decision={decision:?}")
        }
        EventBody::StatusTransitioned { target, forced, .. } => {
            format!("target={} forced={forced}", target.as_str())
        }
        EventBody::EventSuperseded { target, .. } => format!("superseded {target}"),
        _ => String::new(),
    }
}

/// Silence clippy about unused imports we use only in doctests.
#[doc(hidden)]
pub fn _keepalive(_: Timestamp) {
    let _ = Cost::default();
}

#[cfg(test)]
mod tests {
    use knotch_kernel::{UnitId, event::CommitRef};

    use super::*;

    #[test]
    fn session_emits_agent_principal() {
        let session = Session::new("alice", "claude-opus-4-7", "claude-code/1.0");
        let causation = session.tool("edit_file", "inv-1");
        assert!(matches!(causation.principal, Principal::Agent { .. }));
        assert_eq!(causation.session, Some(session.id()));
    }

    #[test]
    fn required_phases_is_four() {
        assert_eq!(Vibe.required_phases(&Scope::Standard).len(), 4);
    }

    #[test]
    fn summary_budget_caps_output_length() {
        let unit = UnitId::try_new("x").unwrap();
        let log: Log<Vibe> = Log::empty(unit);
        let summary = summary_for_llm(&log, SummaryBudget { max_tokens: 64 });
        assert!(summary.body.len() <= 64 * 4 + 128);
    }

    #[test]
    fn event_tag_covers_common_bodies() {
        let body: EventBody<Vibe> = EventBody::UnitCreated { scope: Scope::Standard };
        assert_eq!(event_tag(&body), "unit_created");
        let body: EventBody<Vibe> = EventBody::MilestoneShipped {
            milestone: TaskId("x".into()),
            commit: CommitRef::new("a"),
            commit_kind: knotch_kernel::event::CommitKind::Feat,
            status: knotch_kernel::event::CommitStatus::Verified,
        };
        assert_eq!(event_tag(&body), "milestone_shipped");
    }
}
