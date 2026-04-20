//! Per-body precondition matrix — one pass/fail case per variant.

#![allow(missing_docs)]

use std::{borrow::Cow, num::NonZeroU32, path::Path};

use jiff::Timestamp;
use knotch_kernel::{
    Causation, CommitStatus, Decision, EventId, Log, PhaseKind, Proposal, Rationale,
    RepositoryError, Scope, StatusId, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{
        ArtifactList, CommitKind, CommitRef, EventBody, FailureKind, RetryAnchor,
        SkipKind,
    },
    precondition::{AppendContext, ArtifactCheck, VerifyCommit},
    error::PreconditionError,
};
use serde::{Deserialize, Serialize};

// --- Workflow fixture -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum P { One, Two }

impl PhaseKind for P {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self { P::One => "one", P::Two => "two" })
    }
    fn is_skippable(&self, r: &SkipKind) -> bool {
        matches!(r, SkipKind::ScopeTooNarrow) && matches!(self, P::Two)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct M(String);
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed(self.0.as_str()) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct G(String);
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed(self.0.as_str()) }
}

#[derive(Debug, Clone, Copy, Default)]
struct Wf;
const PHASES: [P; 2] = [P::One, P::Two];
impl WorkflowKind for Wf {
    type Phase = P;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("precondition-fixture") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

// --- Helpers ----------------------------------------------------------

fn causation() -> Causation {
    Causation::new(Source::Cli, Principal::System { service: "t".into() }, Trigger::Manual)
}

fn proposal(body: EventBody<Wf>) -> Proposal<Wf> {
    Proposal { causation: causation(), extension: (), body, supersedes: None }
}

fn log(events: Vec<EventBody<Wf>>) -> Log<Wf> {
    // Construct synthetic events with monotonic UUIDv7 ids + fresh now
    // timestamps. The preconditions we're testing inspect body shape
    // and log structure — ids/timestamps are inert.
    let events = events
        .into_iter()
        .map(|body| knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body,
            supersedes: None,
        })
        .collect();
    Log::from_events(UnitId::new("u"), events)
}

const WF: Wf = Wf;

fn ctx(log: &Log<Wf>) -> AppendContext<'_, Wf> {
    AppendContext::new(&WF, log)
}

// --- UnitCreated ------------------------------------------------------

#[test]
fn unit_created_passes_on_empty_log() {
    let l = log(vec![]);
    let body: EventBody<Wf> = EventBody::UnitCreated { scope: Scope::Standard };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

#[test]
fn unit_created_rejects_on_non_empty_log() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::UnitCreated { scope: Scope::Standard };
    assert_eq!(body.check_precondition(&ctx(&l)), Err(PreconditionError::AlreadyCreated));
}

// --- PhaseCompleted ---------------------------------------------------

#[test]
fn phase_completed_rejects_when_phase_already_completed() {
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() },
    ]);
    let body: EventBody<Wf> = EventBody::PhaseCompleted {
        phase: P::One,
        artifacts: ArtifactList::default(),
    };
    assert_eq!(
        body.check_precondition(&ctx(&l)),
        Err(PreconditionError::PhaseAlreadyCompleted("one".into())),
    );
}

#[test]
fn phase_completed_requires_artifacts_when_fs_provided() {
    struct MissingFs;
    impl ArtifactCheck for MissingFs {
        fn exists(&self, _: &Path) -> bool { false }
    }
    let _unit = UnitId::new("u");
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let mut artifacts = ArtifactList::default();
    artifacts.0.push("plan.md".into());
    let body: EventBody<Wf> = EventBody::PhaseCompleted { phase: P::One, artifacts };
    let ctx = AppendContext::new(&WF, &l).with_fs(&MissingFs);
    assert_eq!(
        body.check_precondition(&ctx),
        Err(PreconditionError::ArtifactMissing { path: "plan.md".into() }),
    );
}

// --- PhaseSkipped -----------------------------------------------------

#[test]
fn phase_skipped_respects_is_skippable() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    // P::One refuses skips.
    let body: EventBody<Wf> = EventBody::PhaseSkipped {
        phase: P::One,
        reason: SkipKind::ScopeTooNarrow,
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::SkipRejected { .. }));

    // P::Two accepts ScopeTooNarrow.
    let body: EventBody<Wf> = EventBody::PhaseSkipped {
        phase: P::Two,
        reason: SkipKind::ScopeTooNarrow,
    };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

// --- MilestoneShipped --------------------------------------------------

#[test]
fn milestone_shipped_rejects_non_implementation_kind() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::MilestoneShipped {
        milestone: M("x".into()),
        commit: CommitRef::new("deadbee"),
        commit_kind: CommitKind::Docs,
        status: CommitStatus::Verified,
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::CommitKindNotImplementation { .. }));
}

#[test]
fn milestone_shipped_uses_vcs_when_provided() {
    struct Missing;
    impl VerifyCommit for Missing {
        fn verify(&self, _: &CommitRef) -> Result<CommitStatus, PreconditionError> {
            Ok(CommitStatus::Missing)
        }
    }
    let _unit = UnitId::new("u");
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::MilestoneShipped {
        milestone: M("x".into()),
        commit: CommitRef::new("deadbee"),
        commit_kind: CommitKind::Feat,
        status: CommitStatus::Verified,
    };
    let ctx = AppendContext::new(&WF, &l).with_vcs(&Missing);
    let err = body.check_precondition(&ctx).unwrap_err();
    assert!(matches!(err, PreconditionError::CommitUnverifiable(_)));
}

// --- MilestoneReverted ------------------------------------------------

#[test]
fn milestone_reverted_requires_prior_ship() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::MilestoneReverted {
        milestone: M("x".into()),
        original: CommitRef::new("a"),
        revert: CommitRef::new("b"),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::MilestoneNotShipped(_)));
}

// --- MilestoneVerified ------------------------------------------------

#[test]
fn milestone_verified_requires_pending_ship() {
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::MilestoneShipped {
            milestone: M("x".into()),
            commit: CommitRef::new("abc"),
            commit_kind: CommitKind::Feat,
            status: CommitStatus::Pending,
        },
    ]);
    let body: EventBody<Wf> = EventBody::MilestoneVerified {
        milestone: M("x".into()),
        commit: CommitRef::new("abc"),
    };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

#[test]
fn milestone_verified_rejects_when_no_pending() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::MilestoneVerified {
        milestone: M("x".into()),
        commit: CommitRef::new("abc"),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NoPendingShip { .. }));
}

// --- GateRecorded -----------------------------------------------------

#[test]
fn gate_recorded_enforces_min_rationale() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    // Rationale::new already enforces DEFAULT_MIN_RATIONALE_CHARS = 8
    // so a too-short rationale cannot even be constructed. This test
    // pins the default-permissive path.
    let body: EventBody<Wf> = EventBody::GateRecorded {
        gate: G("g0".into()),
        decision: Decision::Approved,
        rationale: Rationale::new("good enough rationale").expect("construct"),
    };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

// --- Gate ordering (kernel-enforced via `GateKind::prerequisites`) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum OG {
    A,
    B,
    C,
}

impl knotch_kernel::GateKind for OG {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            OG::A => "a",
            OG::B => "b",
            OG::C => "c",
        })
    }
    fn prerequisites(&self) -> Cow<'_, [Self]> {
        match self {
            OG::A => Cow::Borrowed(&[]),
            OG::B => Cow::Borrowed(&OG_PREREQS_B),
            OG::C => Cow::Borrowed(&OG_PREREQS_C),
        }
    }
}

const OG_PREREQS_B: [OG; 1] = [OG::A];
const OG_PREREQS_C: [OG; 2] = [OG::A, OG::B];

#[derive(Debug, Clone, Copy, Default)]
struct OrderedWf;
const OPHASES: [P; 1] = [P::One];
impl WorkflowKind for OrderedWf {
    type Phase = P;
    type Milestone = M;
    type Gate = OG;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("ordered-gate-fixture") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&OPHASES) }
}

fn olog(events: Vec<EventBody<OrderedWf>>) -> Log<OrderedWf> {
    let events = events
        .into_iter()
        .map(|body| knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body,
            supersedes: None,
        })
        .collect();
    Log::from_events(UnitId::new("u"), events)
}

const OWF: OrderedWf = OrderedWf;

fn octx(log: &Log<OrderedWf>) -> AppendContext<'_, OrderedWf> {
    AppendContext::new(&OWF, log)
}

#[test]
fn gate_recorded_accepts_first_gate_with_empty_prerequisites() {
    let l = olog(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<OrderedWf> = EventBody::GateRecorded {
        gate: OG::A,
        decision: Decision::Approved,
        rationale: Rationale::new("first gate is allowed").expect("construct"),
    };
    assert!(body.check_precondition(&octx(&l)).is_ok());
}

#[test]
fn gate_recorded_rejects_out_of_order() {
    let l = olog(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<OrderedWf> = EventBody::GateRecorded {
        gate: OG::B,
        decision: Decision::Approved,
        rationale: Rationale::new("B without prior A is rejected").expect("construct"),
    };
    let err = body.check_precondition(&octx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::GateOutOfOrder { .. }));
}

#[test]
fn gate_recorded_accepts_when_all_prerequisites_on_log() {
    let l = olog(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::GateRecorded {
            gate: OG::A,
            decision: Decision::Approved,
            rationale: Rationale::new("first gate recorded").expect("construct"),
        },
        EventBody::GateRecorded {
            gate: OG::B,
            decision: Decision::Approved,
            rationale: Rationale::new("second gate recorded").expect("construct"),
        },
    ]);
    let body: EventBody<OrderedWf> = EventBody::GateRecorded {
        gate: OG::C,
        decision: Decision::Approved,
        rationale: Rationale::new("third gate with both prereqs present").expect("construct"),
    };
    assert!(body.check_precondition(&octx(&l)).is_ok());
}

#[test]
fn gate_recorded_out_of_order_reports_first_missing_prerequisite() {
    let l = olog(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<OrderedWf> = EventBody::GateRecorded {
        gate: OG::C,
        decision: Decision::Approved,
        rationale: Rationale::new("C without A or B").expect("construct"),
    };
    match body.check_precondition(&octx(&l)).unwrap_err() {
        PreconditionError::GateOutOfOrder { gate, missing } => {
            assert!(gate.contains("C"));
            assert!(missing.contains("A"));
        }
        other => panic!("expected GateOutOfOrder, got {other:?}"),
    }
}

// --- StatusTransitioned -----------------------------------------------

#[test]
fn status_transitioned_rejects_noop() {
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::StatusTransitioned {
            target: StatusId::new("draft"),
            forced: false,
            rationale: None,
        },
    ]);
    let body: EventBody<Wf> = EventBody::StatusTransitioned {
        target: StatusId::new("draft"),
        forced: false,
        rationale: None,
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NoOpStatusTransition(_)));
}

// --- Phase × Status cross-invariant ----------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct TerminalWf;
const ALL: [P; 2] = [P::One, P::Two];
impl WorkflowKind for TerminalWf {
    type Phase = P;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("terminal-fixture") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&ALL) }
    fn is_terminal_status(&self, status: &StatusId) -> bool {
        status.as_str() == "archived"
    }
}

const TWF: TerminalWf = TerminalWf;

fn tlog(events: Vec<EventBody<TerminalWf>>) -> Log<TerminalWf> {
    let events = events
        .into_iter()
        .map(|body| knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body,
            supersedes: None,
        })
        .collect();
    Log::from_events(UnitId::new("u"), events)
}

#[test]
fn terminal_transition_rejected_when_required_phases_unresolved() {
    let _unit = UnitId::new("u");
    let l = tlog(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<TerminalWf> = EventBody::StatusTransitioned {
        target: StatusId::new("archived"),
        forced: false,
        rationale: None,
    };
    let ctx = AppendContext::<TerminalWf>::new(&TWF, &l);
    let err = body.check_precondition(&ctx).unwrap_err();
    assert!(matches!(err, PreconditionError::RequiredPhaseNotResolved { .. }));
}

#[test]
fn terminal_transition_accepted_when_all_phases_resolved() {
    let _unit = UnitId::new("u");
    let l = tlog(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() },
        EventBody::PhaseCompleted { phase: P::Two, artifacts: ArtifactList::default() },
    ]);
    let body: EventBody<TerminalWf> = EventBody::StatusTransitioned {
        target: StatusId::new("archived"),
        forced: false,
        rationale: None,
    };
    let ctx = AppendContext::<TerminalWf>::new(&TWF, &l);
    assert!(body.check_precondition(&ctx).is_ok());
}

#[test]
fn forced_terminal_transition_bypasses_cross_invariant() {
    let _unit = UnitId::new("u");
    let l = tlog(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<TerminalWf> = EventBody::StatusTransitioned {
        target: StatusId::new("archived"),
        forced: true,
        rationale: Some(Rationale::new("escape hatch reason").unwrap()),
    };
    let ctx = AppendContext::<TerminalWf>::new(&TWF, &l);
    assert!(body.check_precondition(&ctx).is_ok());
}

#[test]
fn status_transitioned_forced_requires_rationale() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::StatusTransitioned {
        target: StatusId::new("archived"),
        forced: true,
        rationale: None,
    };
    assert_eq!(
        body.check_precondition(&ctx(&l)),
        Err(PreconditionError::ForcedWithoutRationale),
    );
}

// --- ReconcileFailed / Recovered --------------------------------------

#[test]
fn reconcile_failed_requires_strict_monotonic_attempt() {
    let anchor = RetryAnchor::Commit { sha: CommitRef::new("sha") };
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::ReconcileFailed {
            anchor: anchor.clone(),
            kind: FailureKind::Unknown,
            attempt: NonZeroU32::new(2).unwrap(),
        },
    ]);
    let body: EventBody<Wf> = EventBody::ReconcileFailed {
        anchor: anchor.clone(),
        kind: FailureKind::Unknown,
        attempt: NonZeroU32::new(2).unwrap(),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NonMonotonicAttempt { .. }));

    let body: EventBody<Wf> = EventBody::ReconcileFailed {
        anchor,
        kind: FailureKind::Unknown,
        attempt: NonZeroU32::new(3).unwrap(),
    };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

#[test]
fn reconcile_recovered_requires_prior_failure() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::ReconcileRecovered {
        anchor: RetryAnchor::Commit { sha: CommitRef::new("sha") },
        attempts_total: NonZeroU32::new(1).unwrap(),
    };
    assert_eq!(body.check_precondition(&ctx(&l)), Err(PreconditionError::NoPriorFailure));
}

// --- EventSuperseded --------------------------------------------------

#[test]
fn event_superseded_rejects_missing_target() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> = EventBody::EventSuperseded {
        target: EventId::new_v7(),
        reason: Rationale::new("rollback for reasons").unwrap(),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::SupersedeTargetMissing(_)));
}

/// Double-supersede of the same target must be rejected. This is
/// the structural guarantee that rules out `A → B → A`-style cycles:
/// once an event has been superseded, no later event can re-
/// supersede it, and a new event cannot reach back into the past to
/// supersede an event that hasn't yet been appended. Append-only
/// construction combined with this check makes supersede cycles
/// impossible by design — no chain-walk guard is required.
#[test]
fn event_superseded_rejects_double_supersede_of_same_target() {
    let unit = UnitId::new("u");
    // Build two events directly so we can grab the target id.
    let base_id = EventId::new_v7();
    let base = knotch_kernel::Event {
        id: base_id,
        at: Timestamp::now(),
        causation: causation(),
        extension: (),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    let phase = knotch_kernel::Event {
        id: EventId::new_v7(),
        at: Timestamp::now(),
        causation: causation(),
        extension: (),
        body: EventBody::PhaseCompleted {
            phase: P::One,
            artifacts: ArtifactList::default(),
        },
        supersedes: None,
    };
    let target = phase.id;
    let first_supersede = knotch_kernel::Event {
        id: EventId::new_v7(),
        at: Timestamp::now(),
        causation: causation(),
        extension: (),
        body: EventBody::EventSuperseded {
            target,
            reason: Rationale::new("first rollback").unwrap(),
        },
        supersedes: None,
    };
    let l = Log::<Wf>::from_events(unit.clone(), vec![base, phase, first_supersede]);

    // Second attempt to supersede the same target must be rejected.
    let body: EventBody<Wf> = EventBody::EventSuperseded {
        target,
        reason: Rationale::new("second rollback").unwrap(),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(
        matches!(err, PreconditionError::AlreadySuperseded(_)),
        "expected AlreadySuperseded, got {err:?}",
    );
    // base_id is unused in this assertion but is retained so the
    // regression test matches the narrative in the docstring.
    let _ = base_id;
}

// --- Repository-level integration via InMemoryRepository --------------

#[tokio::test]
async fn repository_rejects_already_created() {
    let repo = knotch_testing::InMemoryRepository::<Wf>::new(Wf);
    let unit = UnitId::new("u");
    use knotch_kernel::{AppendMode, Repository};
    repo.append(&unit, vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
                AppendMode::BestEffort).await.unwrap();
    let report = repo.append(
        &unit,
        vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    ).await.unwrap();
    // Duplicate dedup wins over precondition — idempotent replay is silent.
    assert_eq!(report.rejected.len(), 1);
    assert!(report.accepted.is_empty());

    // But a fresh-fingerprint UnitCreated with different scope still
    // surfaces AlreadyCreated.
    let body = EventBody::UnitCreated { scope: Scope::Tiny };
    let report = repo.append(&unit, vec![proposal(body)], AppendMode::BestEffort)
        .await.unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("already"));
}

#[tokio::test]
async fn repository_all_or_nothing_propagates_precondition_error() {
    let repo = knotch_testing::InMemoryRepository::<Wf>::new(Wf);
    let unit = UnitId::new("u");
    use knotch_kernel::{AppendMode, Repository};
    // PhaseCompleted without UnitCreated is valid (the current
    // precondition set doesn't require UnitCreated as a prior), but a
    // second PhaseCompleted(One) after one already completed must fail.
    repo.append(
        &unit,
        vec![
            proposal(EventBody::UnitCreated { scope: Scope::Standard }),
            proposal(EventBody::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() }),
        ],
        AppendMode::BestEffort,
    ).await.unwrap();

    let err = repo.append(
        &unit,
        vec![proposal(EventBody::PhaseCompleted {
            phase: P::One,
            artifacts: {
                // Fingerprint must differ — use a non-empty list so the
                // precondition path runs instead of dedup.
                let mut a = ArtifactList::default();
                a.0.push("x.md".into());
                a
            },
        })],
        AppendMode::AllOrNothing,
    ).await.unwrap_err();
    assert!(matches!(err, RepositoryError::Precondition(PreconditionError::PhaseAlreadyCompleted(_))));
}

// --- P0-1 terminal-unit append refusal ---------------------------------

/// Once a unit reaches a terminal status, every variant except
/// `EventSuperseded` is refused so archived / abandoned units stay
/// immutable. `EventSuperseded` is the explicit escape hatch for
/// reverting a mistaken transition.
#[test]
fn terminal_unit_refuses_non_supersede_appends() {
    // Define a workflow whose `archived` status is terminal.
    #[derive(Debug, Clone, Copy)]
    struct Terminal;
    impl WorkflowKind for Terminal {
        type Phase = P;
        type Milestone = M;
        type Gate = G;
        type Extension = ();
        fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("terminal-fixture") }
        fn schema_version(&self) -> u32 { 1 }
        fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&[]) }
        fn is_terminal_status(&self, status: &StatusId) -> bool {
            status.as_str() == "archived"
        }
    }

    // Seed a log that has already transitioned to `archived`.
    let unit = UnitId::new("u");
    let log: Log<Terminal> = Log::from_events(
        unit.clone(),
        vec![
            knotch_kernel::Event {
                id: EventId::new_v7(),
                at: Timestamp::now(),
                causation: causation(),
                extension: (),
                body: EventBody::UnitCreated { scope: Scope::Standard },
                supersedes: None,
            },
            knotch_kernel::Event {
                id: EventId::new_v7(),
                at: Timestamp::now(),
                causation: causation(),
                extension: (),
                body: EventBody::StatusTransitioned {
                    target: StatusId::new("archived"),
                    forced: true,
                    rationale: Some(Rationale::new("upstream dropped").unwrap()),
                },
                supersedes: None,
            },
        ],
    );
    let tw = Terminal;
    let ctx = AppendContext::new(&tw, &log);

    // A plain PhaseCompleted must be refused.
    let err = EventBody::<Terminal>::PhaseCompleted {
        phase: P::One,
        artifacts: ArtifactList::default(),
    }
    .check_precondition(&ctx)
    .unwrap_err();
    assert!(
        matches!(err, PreconditionError::AppendAgainstTerminalUnit { .. }),
        "expected AppendAgainstTerminalUnit, got {err:?}",
    );

    // EventSuperseded is the escape hatch and must still be allowed
    // past the terminal check (its own "target exists" check will
    // then decide admissibility).
    let target = log.events()[1].id;
    let supersede_err = EventBody::<Terminal>::EventSuperseded {
        target,
        reason: Rationale::new("undo archive").unwrap(),
    }
    .check_precondition(&ctx);
    assert!(
        supersede_err.is_ok(),
        "supersede must be allowed on a terminal unit, got {supersede_err:?}",
    );
}
