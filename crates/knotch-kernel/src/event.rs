//! Event envelope and body enumeration.

use std::num::NonZeroU32;

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use crate::{
    causation::{AgentId, Causation, ModelId, Person},
    id::EventId,
    rationale::Rationale,
    scope::Scope,
    status::{Decision, StatusId},
    time::Timestamp,
    workflow::WorkflowKind,
};

/// Top-level event envelope. Metadata (`id`, `at`, `causation`,
/// `extension`, `supersedes`) sits on the envelope so every
/// [`EventBody`] variant carries it uniformly — addressing the
/// "extension only on Gate" asymmetry of v0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                 W::Gate: Serialize, W::Extension: Serialize",
    deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                   W::Milestone: serde::de::DeserializeOwned, \
                   W::Gate: serde::de::DeserializeOwned, \
                   W::Extension: serde::de::DeserializeOwned"
))]
pub struct Event<W: WorkflowKind> {
    /// UUIDv7 event id — time-sortable, OTel-compatible.
    pub id: EventId,
    /// Nanosecond timestamp; the Repository rejects non-monotonic appends.
    pub at: Timestamp,
    /// Attribution chain.
    pub causation: Causation,
    /// Workflow-specific typed payload.
    pub extension: W::Extension,
    /// Body variant.
    pub body: EventBody<W>,
    /// Non-destructive rollback linkage. `None` for the common case;
    /// chains are allowed: A→B→C.
    pub supersedes: Option<EventId>,
}

/// Proposal — an `Event<W>` without id / timestamp. Observers and CLI
/// commands produce these; the `Repository::append` path stamps them
/// and assigns a fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                 W::Gate: Serialize, W::Extension: Serialize",
    deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                   W::Milestone: serde::de::DeserializeOwned, \
                   W::Gate: serde::de::DeserializeOwned, \
                   W::Extension: serde::de::DeserializeOwned"
))]
pub struct Proposal<W: WorkflowKind> {
    /// Attribution chain.
    pub causation: Causation,
    /// Workflow-specific typed payload.
    pub extension: W::Extension,
    /// Body variant.
    pub body: EventBody<W>,
    /// Optional supersede target.
    pub supersedes: Option<EventId>,
}

/// Event body — the sealed taxonomy of knotch mutations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    bound(
        serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                     W::Gate: Serialize, W::Extension: Serialize",
        deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                       W::Milestone: serde::de::DeserializeOwned, \
                       W::Gate: serde::de::DeserializeOwned, \
                       W::Extension: serde::de::DeserializeOwned"
    )
)]
#[non_exhaustive]
pub enum EventBody<W: WorkflowKind> {
    /// The unit has been created. Exactly one per log.
    UnitCreated {
        /// Chosen scope — fixes `required_phases` for this unit.
        scope: Scope,
    },
    /// A phase has been completed with the required artifacts present.
    PhaseCompleted {
        /// The phase that completed.
        phase: W::Phase,
        /// Artifacts that satisfy the phase contract.
        artifacts: ArtifactList,
    },
    /// A phase has been explicitly skipped. Phase must be skippable
    /// for the given reason.
    PhaseSkipped {
        /// The phase skipped.
        phase: W::Phase,
        /// Why — guarded by `PhaseKind::is_skippable`.
        reason: SkipKind,
    },
    /// A milestone has shipped in a commit.
    MilestoneShipped {
        /// Milestone identity.
        milestone: W::Milestone,
        /// Commit that shipped it.
        commit: CommitRef,
        /// Commit-kind classification (conventional-commits grammar).
        commit_kind: CommitKind,
        /// Visibility of the commit at ship time.
        ///
        /// `Verified` is the common case. `Pending` models commits
        /// that exist in a referenced context (remote branch not yet
        /// fetched) but aren't locally visible yet — a later
        /// reconcile pass emits `MilestoneVerified` to promote them.
        /// `Missing` is rejected at the precondition layer and never
        /// reaches the log.
        status: CommitStatus,
    },
    /// A previously-shipped milestone has been reverted.
    MilestoneReverted {
        /// Milestone identity.
        milestone: W::Milestone,
        /// The original shipping commit.
        original: CommitRef,
        /// The revert commit.
        revert: CommitRef,
    },
    /// A pending-status commit has been promoted to verified.
    MilestoneVerified {
        /// Milestone identity.
        milestone: W::Milestone,
        /// Commit that was verified.
        commit: CommitRef,
    },
    /// A gate has been recorded with a decision and rationale.
    GateRecorded {
        /// Gate identity.
        gate: W::Gate,
        /// Decision value.
        decision: Decision,
        /// Non-empty rationale.
        rationale: Rationale,
    },
    /// The unit has been explicitly transitioned to a new status.
    StatusTransitioned {
        /// New status id.
        target: StatusId,
        /// Forced transition bypasses the Phase × Status cross-
        /// invariant and requires a rationale.
        forced: bool,
        /// Rationale (required when `forced`).
        rationale: Option<Rationale>,
    },
    /// A reconcile pass failed; retried under the anchor.
    ReconcileFailed {
        /// What is being retried.
        anchor: RetryAnchor,
        /// Failure classification.
        kind: FailureKind,
        /// Monotonic attempt counter.
        attempt: NonZeroU32,
    },
    /// A previously-failed retry has succeeded.
    ReconcileRecovered {
        /// Matches a prior `ReconcileFailed` anchor.
        anchor: RetryAnchor,
        /// Total attempts including the successful one.
        attempts_total: NonZeroU32,
    },
    /// A prior event has been superseded. Non-destructive — the
    /// target event remains in the log; projections apply
    /// `effective_events` to skip superseded entries.
    EventSuperseded {
        /// Event being superseded.
        target: EventId,
        /// Why — minimum-length rationale.
        reason: Rationale,
    },
    /// A subagent (a task delegated from the main agent session)
    /// finished. Carries enough metadata to reconstruct "who
    /// delegated, of which type, with which last-visible output"
    /// without inflating the log with per-token events — Claude Code
    /// fires `SubagentStop` once per subagent, and the parent
    /// session's `Causation.principal.agent_id` identifies the
    /// delegator implicitly via adjacency in the event stream.
    ///
    /// Appended via `knotch-agent::subagent::record` from the
    /// `SubagentStop` hook wrapper. The prior version of that helper
    /// wrote a side-channel `.knotch/subagents/<id>.json` file; that
    /// path is retired — §I "event log is the only truth" applies to
    /// subagent bookkeeping too.
    SubagentCompleted {
        /// Harness-assigned subagent id (stable per delegated task).
        agent_id: AgentId,
        /// Subagent type tag (`"Explore"`, `"Plan"`, or an adopter-
        /// chosen custom name). `None` when the harness did not
        /// classify the subagent.
        agent_type: Option<CompactString>,
        /// Absolute path to the subagent's transcript JSONL when the
        /// harness produced one. Kept as a string (not `PathBuf`) so
        /// the log stays portable across Linux / macOS / Windows
        /// path-separator conventions.
        transcript_path: Option<CompactString>,
        /// Last assistant message emitted by the subagent. Capped by
        /// the harness; `None` when nothing was reported.
        last_message: Option<CompactString>,
    },
    /// A tool invocation has failed. Separate from `ReconcileFailed`
    /// (which covers reconciler-owned retries under a
    /// `RetryAnchor::Observer`) — this variant attributes failures to
    /// a concrete `(tool, call_id)` pair the agent is driving, so
    /// retry topology is inspectable by projections and by
    /// `knotch-query` (future: filter by failure reason).
    ///
    /// Precondition: `attempt` must be strictly greater than every
    /// prior `ToolCallFailed` with the same `(tool, call_id)` — the
    /// log encodes retry order monotonically.
    ToolCallFailed {
        /// Tool name (e.g. `"Bash"`, `"Edit"`, `"WebFetch"`).
        tool: CompactString,
        /// Harness-assigned tool-call correlation id. Stable across
        /// retries of the same logical call.
        call_id: CompactString,
        /// 1-indexed attempt counter. Monotonic per `(tool, call_id)`.
        attempt: NonZeroU32,
        /// Structured failure classification. Drives automated retry
        /// decisions downstream (observers can branch on
        /// `RateLimited { retry_after }` vs `Timeout { ... }`).
        reason: FailureReason,
    },
    /// The active model for subsequent events has switched. Fires
    /// from the agent harness when it transitions between LLMs
    /// mid-unit (e.g. `opus → haiku` after context compaction, or
    /// `haiku → sonnet` on a cost-budget trigger). Enables the
    /// per-model cost/attribution roll-up in `project::model_timeline`
    /// without forcing every consumer to walk causation by hand.
    ///
    /// Precondition: `to != from`. A no-op switch would inflate
    /// the log and defeat the timeline projection.
    ModelSwitched {
        /// The model that was active immediately before this event.
        from: ModelId,
        /// The model that becomes active from this event onward.
        to: ModelId,
    },
    /// A human (operator, reviewer, manager) has recorded an
    /// approval — or rejection — of a specific prior event. Shipped
    /// for human-in-the-loop workflows where an agent proposes an
    /// action (a gate decision, a status transition, a milestone
    /// push) and a named person ratifies or refuses it.
    ///
    /// Preconditions:
    /// - `target` must reference an event that exists in the log.
    /// - The same `approver` must not already have recorded an approval for the same
    ///   `target` — duplicate signatures are meaningless. A different approver may always
    ///   chime in.
    /// - `Rationale` already enforces the `W::min_rationale_chars()` floor at
    ///   construction, so the signature is meaningfully documented.
    ///
    /// Emitted by the `knotch approve` CLI subcommand — see
    /// `plugins/knotch/skills/knotch-approve/SKILL.md`.
    ApprovalRecorded {
        /// The event being approved or refused.
        target: EventId,
        /// Named human who is signing off (or refusing). `Person`
        /// carries `Sensitive` — subscribers hash it for external
        /// sinks.
        approver: Person,
        /// The decision carried with the signature. Reuses the same
        /// `Decision` vocabulary as `GateRecorded` so dashboards
        /// have a single enum to aggregate across both surfaces.
        decision: Decision,
        /// Why — bounded by `W::min_rationale_chars()`.
        rationale: Rationale,
    },
}

impl<W: WorkflowKind> EventBody<W> {
    /// Canonical snake-case tag for the body variant.
    ///
    /// Single source of truth for `tracing` attributes, reconciler
    /// sort keys, and any place that otherwise wanted a wildcard
    /// `_ => "unknown"` match arm. The match is written in-crate —
    /// kernel itself can exhaustively enumerate its own sealed enum
    /// even though the type is `#[non_exhaustive]` to downstream
    /// crates.
    #[must_use]
    pub fn kind_tag(&self) -> &'static str {
        match self {
            EventBody::UnitCreated { .. } => "unit_created",
            EventBody::PhaseCompleted { .. } => "phase_completed",
            EventBody::PhaseSkipped { .. } => "phase_skipped",
            EventBody::MilestoneShipped { .. } => "milestone_shipped",
            EventBody::MilestoneReverted { .. } => "milestone_reverted",
            EventBody::MilestoneVerified { .. } => "milestone_verified",
            EventBody::GateRecorded { .. } => "gate_recorded",
            EventBody::StatusTransitioned { .. } => "status_transitioned",
            EventBody::ReconcileFailed { .. } => "reconcile_failed",
            EventBody::ReconcileRecovered { .. } => "reconcile_recovered",
            EventBody::EventSuperseded { .. } => "event_superseded",
            EventBody::SubagentCompleted { .. } => "subagent_completed",
            EventBody::ToolCallFailed { .. } => "tool_call_failed",
            EventBody::ModelSwitched { .. } => "model_switched",
            EventBody::ApprovalRecorded { .. } => "approval_recorded",
        }
    }

    /// Reconciler-precedence ordinal — used to put proposals into a
    /// deterministic append order. The numeric value itself is not
    /// part of the stable surface; callers should treat it as opaque
    /// and only rely on the total-order it induces.
    #[must_use]
    pub fn kind_ordinal(&self) -> u8 {
        match self {
            EventBody::UnitCreated { .. } => 1,
            EventBody::PhaseCompleted { .. } => 2,
            EventBody::PhaseSkipped { .. } => 3,
            EventBody::MilestoneShipped { .. } => 4,
            EventBody::MilestoneReverted { .. } => 5,
            EventBody::MilestoneVerified { .. } => 6,
            EventBody::GateRecorded { .. } => 7,
            EventBody::StatusTransitioned { .. } => 8,
            EventBody::ReconcileFailed { .. } => 9,
            EventBody::ReconcileRecovered { .. } => 10,
            EventBody::EventSuperseded { .. } => 11,
            EventBody::SubagentCompleted { .. } => 12,
            EventBody::ToolCallFailed { .. } => 13,
            EventBody::ModelSwitched { .. } => 14,
            EventBody::ApprovalRecorded { .. } => 15,
        }
    }

    /// Evaluate the per-variant precondition against `ctx`.
    ///
    /// Returns `Ok(())` if the append is admissible, otherwise a
    /// structured `PreconditionError`. This is the kernel's
    /// policy-enforcement pivot — `FileRepository::append` /
    /// `InMemoryRepository::append` call this method before
    /// dedup-checking or persisting each proposal.
    ///
    /// # Errors
    /// Per-variant; see `PreconditionError` for the full taxonomy.
    pub fn check_precondition(
        &self,
        ctx: &crate::precondition::AppendContext<'_, W>,
    ) -> Result<(), crate::error::PreconditionError> {
        use crate::{
            error::PreconditionError as E,
            project::{current_status, effective_events, shipped_milestones},
            workflow::{MilestoneKind as _, PhaseKind as _},
        };

        // A unit that has reached a terminal status (per
        // `W::is_terminal_status`) accepts no further mutations
        // except `EventSuperseded` — the escape hatch for rolling
        // back a mistaken transition. Every other variant is refused
        // so archived / abandoned / superseded units stay immutable.
        if !matches!(self, EventBody::EventSuperseded { .. })
            && let Some(status) = current_status(ctx.log)
            && ctx.workflow.is_terminal_status(&status)
        {
            return Err(E::AppendAgainstTerminalUnit { status: status.as_str().to_owned() });
        }

        match self {
            EventBody::UnitCreated { .. } => {
                if !ctx.log.events().is_empty() {
                    return Err(E::AlreadyCreated);
                }
            }
            EventBody::PhaseCompleted { phase, artifacts } => {
                for evt in effective_events(ctx.log) {
                    if let EventBody::PhaseCompleted { phase: prior, .. } = &evt.body
                        && prior == phase
                    {
                        return Err(E::PhaseAlreadyCompleted(phase.id().into_owned()));
                    }
                }
                if let Some(fs) = ctx.fs {
                    for path in &artifacts.0 {
                        let p = std::path::Path::new(path.as_str());
                        if !fs.exists(p) {
                            return Err(E::ArtifactMissing { path: path.as_str().to_owned() });
                        }
                    }
                }
            }
            EventBody::PhaseSkipped { phase, reason } => {
                if !ctx.workflow.accepts_skip_for(phase, reason) {
                    return Err(E::SkipRejected {
                        phase: phase.id().into_owned(),
                        reason: format!("{reason:?}"),
                    });
                }
                for evt in effective_events(ctx.log) {
                    if let EventBody::PhaseCompleted { phase: prior, .. }
                    | EventBody::PhaseSkipped { phase: prior, .. } = &evt.body
                        && prior == phase
                    {
                        return Err(E::PhaseAlreadyCompleted(phase.id().into_owned()));
                    }
                }
            }
            EventBody::MilestoneShipped { milestone, commit, commit_kind, status } => {
                if !commit_kind.is_implementation() {
                    return Err(E::CommitKindNotImplementation {
                        kind: format!("{commit_kind:?}"),
                    });
                }
                if matches!(status, CommitStatus::Missing) {
                    return Err(E::CommitUnverifiable(commit.as_str().to_owned()));
                }
                // A milestone may ship only once in the effective log.
                // A revert restores it to the unshipped set, so after
                // `MilestoneReverted` the same milestone becomes
                // re-shippable — but absent a revert, a second
                // `MilestoneShipped` (even with a different commit) is
                // rejected.
                let shipped = shipped_milestones::<W>(ctx.log);
                if shipped.iter().any(|m| m.id() == milestone.id()) {
                    return Err(E::MilestoneAlreadyShipped(milestone.id().into_owned()));
                }
                if let Some(vcs) = ctx.vcs {
                    let observed = vcs.verify(commit)?;
                    match (status, observed) {
                        (_, CommitStatus::Missing) => {
                            return Err(E::CommitUnverifiable(commit.as_str().to_owned()));
                        }
                        (CommitStatus::Verified, CommitStatus::Pending) => {
                            // Caller claimed Verified but VCS says Pending —
                            // policy violation: caller must downgrade the
                            // event or wait.
                            return Err(E::StatusDowngrade {
                                claimed: "verified".into(),
                                observed: "pending".into(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            EventBody::MilestoneReverted { milestone, revert, .. } => {
                let shipped = shipped_milestones::<W>(ctx.log);
                let id = milestone.id();
                if !shipped.iter().any(|m| m.id() == id) {
                    return Err(E::MilestoneNotShipped(id.into_owned()));
                }
                if let Some(vcs) = ctx.vcs
                    && matches!(vcs.verify(revert)?, CommitStatus::Missing)
                {
                    return Err(E::CommitUnverifiable(revert.as_str().to_owned()));
                }
            }
            EventBody::MilestoneVerified { milestone, commit } => {
                // Must correspond to a prior `MilestoneShipped` whose
                // status was Pending and whose commit matches.
                let found = effective_events(ctx.log).iter().any(|evt| match &evt.body {
                    EventBody::MilestoneShipped {
                        milestone: m,
                        commit: c,
                        status: CommitStatus::Pending,
                        ..
                    } => m.id() == milestone.id() && c == commit,
                    _ => false,
                });
                if !found {
                    return Err(E::NoPendingShip {
                        milestone: milestone.id().into_owned(),
                        commit: commit.as_str().to_owned(),
                    });
                }
                if let Some(vcs) = ctx.vcs
                    && !matches!(vcs.verify(commit)?, CommitStatus::Verified)
                {
                    return Err(E::CommitUnverifiable(commit.as_str().to_owned()));
                }
            }
            EventBody::GateRecorded { gate, rationale, .. } => {
                let min = ctx.workflow.min_rationale_chars();
                if rationale.char_len() < min {
                    return Err(E::RationaleTooShort { min, actual: rationale.char_len() });
                }
                let required = ctx.workflow.prerequisites_for(gate);
                if !required.is_empty() {
                    // Supersede-aware: a superseded `GateRecorded`
                    // no longer satisfies the prerequisite chain.
                    let effective = effective_events(ctx.log);
                    let mut recorded: Vec<&W::Gate> = Vec::new();
                    for evt in &effective {
                        if let EventBody::GateRecorded { gate: g, .. } = &evt.body {
                            recorded.push(g);
                        }
                    }
                    for prereq in required.iter() {
                        if !recorded.contains(&prereq) {
                            return Err(E::GateOutOfOrder {
                                gate: format!("{:?}", gate),
                                missing: format!("{:?}", prereq),
                            });
                        }
                    }
                }
            }
            EventBody::StatusTransitioned { target, forced, rationale } => {
                if let Some(current) = crate::project::current_status(ctx.log)
                    && &current == target
                {
                    return Err(E::NoOpStatusTransition(target.clone()));
                }
                if *forced && rationale.is_none() {
                    return Err(E::ForcedWithoutRationale);
                }
                // Phase × Status cross-invariant: terminal
                // transitions require all required phases to be
                // resolved unless the caller explicitly forces.
                if !*forced
                    && ctx.workflow.is_terminal_status(target)
                    && let Some(scope) = scope_of_log(ctx.log)
                {
                    let resolved = phases_resolved(&effective_events(ctx.log));
                    for required in ctx.workflow.required_phases(&scope).iter() {
                        if !resolved.iter().any(|r| r == required) {
                            return Err(E::RequiredPhaseNotResolved {
                                phase: required.id().into_owned(),
                            });
                        }
                    }
                }
            }
            EventBody::ReconcileFailed { anchor, attempt, .. } => {
                let prior = max_attempt_for_anchor(ctx.log, anchor);
                if attempt.get() <= prior {
                    return Err(E::NonMonotonicAttempt { attempt: attempt.get(), prior });
                }
            }
            EventBody::ReconcileRecovered { anchor, .. } => {
                // Supersede-aware: a superseded `ReconcileFailed`
                // is retroactively "not a failure" — recovering
                // from a failure that never effectively happened
                // is meaningless.
                let has_prior = effective_events(ctx.log).iter().any(|evt| {
                    matches!(
                        &evt.body,
                        EventBody::ReconcileFailed { anchor: prior, .. } if prior == anchor
                    )
                });
                if !has_prior {
                    return Err(E::NoPriorFailure);
                }
            }
            EventBody::EventSuperseded { target, .. } => {
                let mut target_exists = false;
                for evt in ctx.log.events() {
                    if evt.id == *target {
                        target_exists = true;
                    }
                    if let EventBody::EventSuperseded { target: prior, .. } = &evt.body
                        && prior == target
                    {
                        return Err(E::AlreadySuperseded(*target));
                    }
                }
                if !target_exists {
                    return Err(E::SupersedeTargetMissing(*target));
                }
            }
            EventBody::SubagentCompleted { agent_id, .. } => {
                // One completion event per subagent id. A duplicate
                // would produce two competing `transcript_path` /
                // `last_message` records under the same `agent_id` and
                // break any projection that assumes "agent X completed
                // at time T".
                let already_completed = effective_events(ctx.log).iter().any(|evt| {
                    matches!(
                        &evt.body,
                        EventBody::SubagentCompleted { agent_id: prior, .. } if prior == agent_id,
                    )
                });
                if already_completed {
                    return Err(E::SubagentAlreadyCompleted(agent_id.clone()));
                }
            }
            EventBody::ToolCallFailed { tool, call_id, attempt, .. } => {
                // Attempt counter must be strictly greater than every
                // prior `ToolCallFailed` for the same (tool, call_id).
                // Otherwise the retry timeline is non-monotonic and
                // projections can't reconstruct attempt order.
                let prior_max =
                    effective_events(ctx.log)
                        .iter()
                        .filter_map(|evt| match &evt.body {
                            EventBody::ToolCallFailed {
                                tool: t, call_id: c, attempt: a, ..
                            } if t == tool && c == call_id => Some(a.get()),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(0);
                if attempt.get() <= prior_max {
                    return Err(E::NonMonotonicAttempt {
                        attempt: attempt.get(),
                        prior: prior_max,
                    });
                }
            }
            EventBody::ModelSwitched { from, to } => {
                // A no-op switch would inflate the log and defeat the
                // `model_timeline` projection — refuse identical
                // `from == to`. The adopter stamps `from` based on
                // whatever tracking it already maintains; knotch
                // doesn't re-validate it against a "current model"
                // projection because that projection is precisely
                // what this event builds.
                if from == to {
                    return Err(E::NoOpModelSwitch { model: from.clone() });
                }
            }
            EventBody::ApprovalRecorded { target, approver, .. } => {
                // Supersede-aware: the target must be an effective
                // event — approving a retracted event is
                // meaningless (the "decision" no longer stands).
                let effective = effective_events(ctx.log);
                let target_present = effective.iter().any(|e| e.id == *target);
                if !target_present {
                    return Err(E::ApprovalTargetMissing(*target));
                }
                // The same approver must not have already signed this
                // target. Different approvers can each record their
                // own approval.
                let duplicate = effective.iter().any(|evt| {
                    matches!(
                        &evt.body,
                        EventBody::ApprovalRecorded { target: t, approver: a, .. }
                            if t == target && a == approver,
                    )
                });
                if duplicate {
                    return Err(E::ApprovalAlreadyRecorded { target: *target });
                }
            }
        }
        Ok(())
    }
}

fn max_attempt_for_anchor<W: WorkflowKind>(log: &crate::log::Log<W>, anchor: &RetryAnchor) -> u32 {
    let mut max = 0;
    for evt in log.events() {
        if let EventBody::ReconcileFailed { anchor: prior, attempt, .. } = &evt.body
            && prior == anchor
            && attempt.get() > max
        {
            max = attempt.get();
        }
    }
    max
}

/// Look up the unit's scope from its `UnitCreated` event.
fn scope_of_log<W: WorkflowKind>(log: &crate::log::Log<W>) -> Option<crate::Scope> {
    log.events().iter().find_map(|evt| match &evt.body {
        EventBody::UnitCreated { scope } => Some(scope.clone()),
        _ => None,
    })
}

/// Collect every phase that has either completed or skipped in the
/// effective log. Used by the Phase × Status cross-invariant.
fn phases_resolved<W: WorkflowKind>(effective: &[crate::Event<W>]) -> Vec<W::Phase> {
    effective
        .iter()
        .filter_map(|evt| match &evt.body {
            EventBody::PhaseCompleted { phase, .. } | EventBody::PhaseSkipped { phase, .. } => {
                Some(phase.clone())
            }
            _ => None,
        })
        .collect()
}

/// Collection of artifact paths that actually existed at the time a
/// phase was marked complete.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactList(pub Vec<CompactString>);

/// Reason a phase was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkipKind {
    /// Scope is narrower than the phase requires (e.g. `Tiny`).
    ScopeTooNarrow,
    /// An explicit amnesty / waiver at the unit level.
    Amnesty {
        /// Short machine-readable reason code.
        code: CompactString,
    },
    /// Workflow-defined custom skip.
    Custom {
        /// Short machine-readable reason code.
        code: CompactString,
    },
}

impl std::str::FromStr for SkipKind {
    type Err = std::convert::Infallible;

    /// Parse a human-supplied skip reason.
    ///
    /// - `"scope_too_narrow"` → [`SkipKind::ScopeTooNarrow`]
    /// - `"amnesty:<code>"`  → [`SkipKind::Amnesty`] with `code`
    /// - anything else       → [`SkipKind::Custom`] with the input as `code`
    ///
    /// Infallible — every input maps to a valid variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "scope_too_narrow" => Self::ScopeTooNarrow,
            rest if rest.starts_with("amnesty:") => {
                Self::Amnesty { code: rest["amnesty:".len()..].into() }
            }
            rest => Self::Custom { code: rest.into() },
        })
    }
}

/// Visibility classification for a commit, shared between the VCS
/// adapter and the event body. Stored on `EventBody::MilestoneShipped`
/// so the log explicitly records whether a shipped commit was
/// `Verified` at write time or still `Pending`. Pending ships are
/// later promoted by `EventBody::MilestoneVerified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommitStatus {
    /// The commit is visible to the verifier.
    Verified,
    /// The commit exists in a referenced context (e.g. a remote
    /// branch not yet fetched) but is not yet locally visible; a
    /// later reconcile pass will promote it to `Verified`.
    Pending,
    /// The commit is unknown. `Missing` is never stored in
    /// `MilestoneShipped` — it is a verifier return value that the
    /// Repository rejects as a precondition failure.
    Missing,
}

/// Conventional-commits classification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommitKind {
    /// `feat:` — new functionality.
    Feat,
    /// `fix:` — defect repair.
    Fix,
    /// `refactor:` — behavior-preserving change.
    Refactor,
    /// `perf:` — performance improvement.
    Perf,
    /// `docs:` — documentation only.
    Docs,
    /// `chore:` — housekeeping.
    Chore,
    /// `test:` — test-only.
    Test,
    /// `ci:` — CI-config only.
    Ci,
    /// `build:` — build-system change.
    Build,
    /// `style:` — whitespace / formatting.
    Style,
    /// `revert:` — reverts a prior commit.
    Revert,
}

impl CommitKind {
    /// Is this commit kind considered an implementation ship? Only
    /// implementation kinds may carry a `MilestoneShipped` event —
    /// `docs` / `chore` / `test` / `ci` / `build` / `style` cannot.
    #[must_use]
    pub const fn is_implementation(&self) -> bool {
        matches!(self, Self::Feat | Self::Fix | Self::Refactor | Self::Perf)
    }
}

/// Commit reference — a 40-char lowercase hex SHA.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitRef(CompactString);

impl CommitRef {
    /// Wrap a commit SHA. Accepts any length; presets can tighten.
    #[must_use]
    pub fn new(sha: impl Into<CompactString>) -> Self {
        Self(sha.into())
    }

    /// Return the SHA as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for CommitRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for CommitRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What is being retried by a `ReconcileFailed` / `ReconcileRecovered`
/// event. Unifies retry accounting across VCS / lock / observer
/// concerns.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetryAnchor {
    /// Retry driven by a specific commit that was not yet verified.
    Commit {
        /// SHA of the pending commit.
        sha: CommitRef,
    },
    /// Retry driven by a stale lock that was reclaimed. `pid` is the
    /// process that had held the lock — an observability anchor, not
    /// a handle.
    Lock {
        /// PID of the prior lock holder at reclaim time.
        pid: u32,
    },
    /// Retry driven by a specific observer error, named by
    /// `Observer::name`.
    Observer {
        /// Observer name (matches `DynObserver::name()`).
        name: CompactString,
    },
}

/// Failure classification for `ReconcileFailed`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureKind {
    /// The commit referenced by a proposal is not yet visible.
    CommitPending,
    /// An observer returned an error.
    ObserverFailed,
    /// A stale lock was reclaimed.
    StaleLockReclaimed,
    /// Unknown / uncategorized.
    Unknown,
}

/// Classification for `ToolCallFailed`. Structured rather than
/// free-form so observers can branch on the failure shape without
/// string matching. `#[non_exhaustive]` — new tool-failure modes
/// land as additive variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureReason {
    /// Rate limit or quota exhausted. `retry_after_secs` is the
    /// server-recommended wait in seconds when provided, `None` when
    /// the harness had no hint.
    RateLimited {
        /// Server-recommended retry delay in seconds, if known.
        retry_after_secs: Option<u64>,
    },
    /// The tool exceeded the caller's deadline.
    Timeout {
        /// Observed elapsed time in seconds at cancellation.
        after_secs: u64,
    },
    /// Backend or network failure — opaque string from the tool /
    /// adapter, not interpretable by knotch.
    Backend {
        /// Short operator-facing error detail.
        message: CompactString,
    },
    /// The user (or a parent agent) cancelled the tool call mid-
    /// flight — not a failure attributable to the tool itself, but
    /// worth logging so cost and retry math is correct.
    UserCancelled,
}

/// Batching policy for `Repository::append`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppendMode {
    /// Every proposal must pass precondition checks or none are
    /// persisted.
    AllOrNothing,
    /// Accept passing proposals, report rejections in the return.
    BestEffort,
}

/// Selects replay semantics for `Repository::subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscribeMode {
    /// Only events appended after the subscription was created.
    LiveOnly,
    /// Replay every event in the log, then continue live.
    FromBeginning,
    /// Replay from a specific event id, then continue live.
    FromEventId(EventId),
}

/// Wrapper delivered on the subscribe stream. `Lagged` surfaces when
/// an overflow policy drops events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                 W::Gate: Serialize, W::Extension: Serialize",
    deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                   W::Milestone: serde::de::DeserializeOwned, \
                   W::Gate: serde::de::DeserializeOwned, \
                   W::Extension: serde::de::DeserializeOwned"
))]
#[non_exhaustive]
pub enum SubscribeEvent<W: WorkflowKind> {
    /// Live or replayed event. Boxed because the `Event<W>` payload
    /// is substantially larger than the `Lagged` variant.
    Event(Box<Event<W>>),
    /// The subscriber was too slow; `skipped` events were dropped,
    /// resuming at `next`.
    Lagged {
        /// Number of events the subscriber lost.
        skipped: u64,
        /// First event id after the gap.
        next: EventId,
    },
}

/// Per-event outcome from `Repository::append`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                 W::Gate: Serialize, W::Extension: Serialize",
    deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                   W::Milestone: serde::de::DeserializeOwned, \
                   W::Gate: serde::de::DeserializeOwned, \
                   W::Extension: serde::de::DeserializeOwned"
))]
#[non_exhaustive]
pub struct AppendReport<W: WorkflowKind> {
    /// Proposals that were appended; the id/timestamp reflect the
    /// Repository-assigned values.
    pub accepted: Vec<Event<W>>,
    /// Proposals that were rejected and why.
    pub rejected: Vec<RejectedProposal<W>>,
}

impl<W: WorkflowKind> AppendReport<W> {
    /// Construct a report. The `#[non_exhaustive]` marker blocks
    /// struct-literal construction outside this crate — adapters
    /// (`knotch-storage`, `knotch-testing`) use this constructor so a
    /// future additive field lands as a new parameter rather than a
    /// silent breakage.
    #[must_use]
    pub fn new(accepted: Vec<Event<W>>, rejected: Vec<RejectedProposal<W>>) -> Self {
        Self { accepted, rejected }
    }

    /// Empty report — no proposals accepted, no rejections.
    #[must_use]
    pub fn empty() -> Self {
        Self { accepted: Vec::new(), rejected: Vec::new() }
    }
}

/// A rejected proposal, along with the reason for rejection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "W: WorkflowKind, W::Phase: Serialize, W::Milestone: Serialize, \
                 W::Gate: Serialize, W::Extension: Serialize",
    deserialize = "W: WorkflowKind, W::Phase: serde::de::DeserializeOwned, \
                   W::Milestone: serde::de::DeserializeOwned, \
                   W::Gate: serde::de::DeserializeOwned, \
                   W::Extension: serde::de::DeserializeOwned"
))]
#[non_exhaustive]
pub struct RejectedProposal<W: WorkflowKind> {
    /// The proposal that was rejected.
    pub proposal: Proposal<W>,
    /// Human-readable reason (typically from `PreconditionError::Display`).
    pub reason: CompactString,
}

impl<W: WorkflowKind> RejectedProposal<W> {
    /// Construct a rejection record. See [`AppendReport::new`] for
    /// the rationale behind the explicit constructor.
    #[must_use]
    pub fn new(proposal: Proposal<W>, reason: impl Into<CompactString>) -> Self {
        Self { proposal, reason: reason.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_kind_implementation_set_matches_spec() {
        // The set matches knotch-v1-final-plan §6.2 precondition matrix.
        let impls: Vec<_> = [
            CommitKind::Feat,
            CommitKind::Fix,
            CommitKind::Refactor,
            CommitKind::Perf,
            CommitKind::Docs,
            CommitKind::Chore,
            CommitKind::Test,
            CommitKind::Ci,
            CommitKind::Build,
            CommitKind::Style,
            CommitKind::Revert,
        ]
        .into_iter()
        .filter(|k| k.is_implementation())
        .collect();
        assert_eq!(
            impls,
            vec![CommitKind::Feat, CommitKind::Fix, CommitKind::Refactor, CommitKind::Perf]
        );
    }
}
