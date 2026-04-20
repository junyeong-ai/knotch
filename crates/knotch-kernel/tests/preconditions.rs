//! Per-body precondition matrix — one pass/fail case per variant.

#![allow(missing_docs)]

use std::{borrow::Cow, num::NonZeroU32, path::Path};

use jiff::Timestamp;
use knotch_kernel::{
    Causation, CommitStatus, Decision, EventId, Log, PhaseKind, Proposal, Rationale,
    RepositoryError, Scope, StatusId, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    error::PreconditionError,
    event::{ArtifactList, CommitKind, CommitRef, EventBody, FailureKind, RetryAnchor, SkipKind},
    precondition::{AppendContext, ArtifactCheck, VerifyCommit},
};
use serde::{Deserialize, Serialize};

// --- Workflow fixture -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum P {
    One,
    Two,
}

impl PhaseKind for P {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            P::One => "one",
            P::Two => "two",
        })
    }
    fn is_skippable(&self, r: &SkipKind) -> bool {
        matches!(r, SkipKind::ScopeTooNarrow) && matches!(self, P::Two)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct M(String);
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct G(String);
impl knotch_kernel::GateKind for G {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Wf;
const PHASES: [P; 2] = [P::One, P::Two];
impl WorkflowKind for Wf {
    type Phase = P;
    type Milestone = M;
    type Gate = G;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("precondition-fixture")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
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
    Log::from_events(UnitId::try_new("u").unwrap(), events)
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
    let body: EventBody<Wf> =
        EventBody::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() };
    assert_eq!(
        body.check_precondition(&ctx(&l)),
        Err(PreconditionError::PhaseAlreadyCompleted("one".into())),
    );
}

#[test]
fn phase_completed_requires_artifacts_when_fs_provided() {
    struct MissingFs;
    impl ArtifactCheck for MissingFs {
        fn exists(&self, _: &Path) -> bool {
            false
        }
    }
    let _unit = UnitId::try_new("u").unwrap();
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
    let body: EventBody<Wf> =
        EventBody::PhaseSkipped { phase: P::One, reason: SkipKind::ScopeTooNarrow };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::SkipRejected { .. }));

    // P::Two accepts ScopeTooNarrow.
    let body: EventBody<Wf> =
        EventBody::PhaseSkipped { phase: P::Two, reason: SkipKind::ScopeTooNarrow };
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
    let _unit = UnitId::try_new("u").unwrap();
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
    let body: EventBody<Wf> =
        EventBody::MilestoneVerified { milestone: M("x".into()), commit: CommitRef::new("abc") };
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

#[test]
fn milestone_verified_rejects_when_no_pending() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body: EventBody<Wf> =
        EventBody::MilestoneVerified { milestone: M("x".into()), commit: CommitRef::new("abc") };
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
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("ordered-gate-fixture")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&OPHASES)
    }
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
    Log::from_events(UnitId::try_new("u").unwrap(), events)
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

#[test]
fn gate_recorded_prerequisite_supersede_retracts_the_gate() {
    // A superseded GateRecorded must no longer satisfy a later
    // gate's prerequisite. Structural guarantee: the precondition
    // walks `effective_events`, not raw log.events().
    let gate_a_id = EventId::new_v7();
    let unit_id = UnitId::try_new("u").unwrap();
    let events: Vec<knotch_kernel::Event<OrderedWf>> = vec![
        knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        },
        knotch_kernel::Event {
            id: gate_a_id,
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::GateRecorded {
                gate: OG::A,
                decision: Decision::Approved,
                rationale: Rationale::new("record A first").unwrap(),
            },
            supersedes: None,
        },
        knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::EventSuperseded {
                target: gate_a_id,
                reason: Rationale::new("retract gate A").unwrap(),
            },
            supersedes: None,
        },
    ];
    let l = Log::from_events(unit_id, events);
    let body: EventBody<OrderedWf> = EventBody::GateRecorded {
        gate: OG::B,
        decision: Decision::Approved,
        rationale: Rationale::new("B after A retracted").unwrap(),
    };
    match body.check_precondition(&octx(&l)).unwrap_err() {
        PreconditionError::GateOutOfOrder { missing, .. } => {
            assert!(missing.contains("A"), "expected A as missing prereq, got {missing}");
        }
        other => panic!("expected GateOutOfOrder after supersede, got {other:?}"),
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
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("terminal-fixture")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&ALL)
    }
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
    Log::from_events(UnitId::try_new("u").unwrap(), events)
}

#[test]
fn terminal_transition_rejected_when_required_phases_unresolved() {
    let _unit = UnitId::try_new("u").unwrap();
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
    let _unit = UnitId::try_new("u").unwrap();
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
    let _unit = UnitId::try_new("u").unwrap();
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
    assert_eq!(body.check_precondition(&ctx(&l)), Err(PreconditionError::ForcedWithoutRationale),);
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
    let unit = UnitId::try_new("u").unwrap();
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
        body: EventBody::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() },
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
    let body: EventBody<Wf> =
        EventBody::EventSuperseded { target, reason: Rationale::new("second rollback").unwrap() };
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
    let unit = UnitId::try_new("u").unwrap();
    use knotch_kernel::{AppendMode, Repository};
    repo.append(
        &unit,
        vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .unwrap();
    let report = repo
        .append(
            &unit,
            vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
            AppendMode::BestEffort,
        )
        .await
        .unwrap();
    // Duplicate dedup wins over precondition — idempotent replay is silent.
    assert_eq!(report.rejected.len(), 1);
    assert!(report.accepted.is_empty());

    // But a fresh-fingerprint UnitCreated with different scope still
    // surfaces AlreadyCreated.
    let body = EventBody::UnitCreated { scope: Scope::Tiny };
    let report = repo.append(&unit, vec![proposal(body)], AppendMode::BestEffort).await.unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("already"));
}

#[tokio::test]
async fn repository_all_or_nothing_propagates_precondition_error() {
    let repo = knotch_testing::InMemoryRepository::<Wf>::new(Wf);
    let unit = UnitId::try_new("u").unwrap();
    use knotch_kernel::{AppendMode, Repository};
    // PhaseCompleted without UnitCreated is valid (the current
    // precondition set doesn't require UnitCreated as a prior), but a
    // second PhaseCompleted(One) after one already completed must fail.
    repo.append(
        &unit,
        vec![
            proposal(EventBody::UnitCreated { scope: Scope::Standard }),
            proposal(EventBody::PhaseCompleted {
                phase: P::One,
                artifacts: ArtifactList::default(),
            }),
        ],
        AppendMode::BestEffort,
    )
    .await
    .unwrap();

    let err = repo
        .append(
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
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        RepositoryError::Precondition(PreconditionError::PhaseAlreadyCompleted(_))
    ));
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
        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed("terminal-fixture")
        }
        fn schema_version(&self) -> u32 {
            1
        }
        fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
            std::borrow::Cow::Borrowed(&[])
        }
        fn is_terminal_status(&self, status: &StatusId) -> bool {
            status.as_str() == "archived"
        }
    }

    // Seed a log that has already transitioned to `archived`.
    let unit = UnitId::try_new("u").unwrap();
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
    let err =
        EventBody::<Terminal>::PhaseCompleted { phase: P::One, artifacts: ArtifactList::default() }
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

// --- SubagentCompleted ------------------------------------------------

#[test]
fn subagent_completed_accepted_first_time() {
    use compact_str::CompactString;
    use knotch_kernel::causation::AgentId;

    let log = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let wf = Wf;
    let ctx = AppendContext::new(&wf, &log);
    EventBody::<Wf>::SubagentCompleted {
        agent_id: AgentId(CompactString::from("agent-abc")),
        agent_type: Some(CompactString::from("Explore")),
        transcript_path: None,
        last_message: None,
    }
    .check_precondition(&ctx)
    .expect("first SubagentCompleted for agent-abc must land");
}

#[test]
fn subagent_completed_rejects_duplicate_agent_id() {
    use compact_str::CompactString;
    use knotch_kernel::causation::AgentId;

    let log = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        EventBody::SubagentCompleted {
            agent_id: AgentId(CompactString::from("agent-abc")),
            agent_type: Some(CompactString::from("Explore")),
            transcript_path: None,
            last_message: None,
        },
    ]);
    let wf = Wf;
    let ctx = AppendContext::new(&wf, &log);
    let err = EventBody::<Wf>::SubagentCompleted {
        agent_id: AgentId(CompactString::from("agent-abc")),
        agent_type: Some(CompactString::from("Plan")),
        transcript_path: Some(CompactString::from("/tmp/transcript.jsonl")),
        last_message: Some(CompactString::from("different second completion")),
    }
    .check_precondition(&ctx)
    .expect_err("duplicate agent_id must reject");
    assert!(
        matches!(err, PreconditionError::SubagentAlreadyCompleted(ref id) if id == "agent-abc")
    );
}

#[test]
fn subagent_completed_after_supersede_can_be_re_recorded() {
    // A superseded SubagentCompleted no longer counts toward the
    // "already completed" check — the agent's prior record has been
    // retracted, so a fresh completion is admissible. Gives operators
    // a clean escape hatch when the first event was wrong (e.g.
    // transcript_path pointed at a rotated-out file).
    use compact_str::CompactString;
    use knotch_kernel::causation::AgentId;

    let mut events = vec![
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
            body: EventBody::SubagentCompleted {
                agent_id: AgentId(CompactString::from("agent-abc")),
                agent_type: None,
                transcript_path: None,
                last_message: None,
            },
            supersedes: None,
        },
    ];
    let original_id = events[1].id;
    events.push(knotch_kernel::Event {
        id: EventId::new_v7(),
        at: Timestamp::now(),
        causation: causation(),
        extension: (),
        body: EventBody::EventSuperseded {
            target: original_id,
            reason: Rationale::new("rotated transcript").unwrap(),
        },
        supersedes: None,
    });
    let log = Log::from_events(UnitId::try_new("u").unwrap(), events);
    let wf = Wf;
    let ctx = AppendContext::new(&wf, &log);
    EventBody::<Wf>::SubagentCompleted {
        agent_id: AgentId(CompactString::from("agent-abc")),
        agent_type: Some(CompactString::from("Explore")),
        transcript_path: Some(CompactString::from("/tmp/new-transcript.jsonl")),
        last_message: None,
    }
    .check_precondition(&ctx)
    .expect("superseded prior allows fresh SubagentCompleted");
}

// --- ApprovalRecorded -------------------------------------------------

#[test]
fn approval_rejects_missing_target() {
    use compact_str::CompactString;
    use knotch_kernel::causation::Person;

    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let phantom = EventId::new_v7();
    let body: EventBody<Wf> = EventBody::ApprovalRecorded {
        target: phantom,
        approver: Person(CompactString::from("alice")),
        decision: Decision::Approved,
        rationale: Rationale::new("looks fine to me").unwrap(),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::ApprovalTargetMissing(_)));
}

#[test]
fn approval_rejects_duplicate_from_same_approver_and_accepts_another() {
    use compact_str::CompactString;
    use knotch_kernel::causation::Person;

    let unit_created_id = EventId::new_v7();
    let events = vec![
        knotch_kernel::Event {
            id: unit_created_id,
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
            body: EventBody::ApprovalRecorded {
                target: unit_created_id,
                approver: Person(CompactString::from("alice")),
                decision: Decision::Approved,
                rationale: Rationale::new("sign-off one").unwrap(),
            },
            supersedes: None,
        },
    ];
    let log = Log::from_events(UnitId::try_new("u").unwrap(), events);
    let duplicate: EventBody<Wf> = EventBody::ApprovalRecorded {
        target: unit_created_id,
        approver: Person(CompactString::from("alice")),
        decision: Decision::Rejected,
        rationale: Rationale::new("changed mind").unwrap(),
    };
    let err = duplicate.check_precondition(&AppendContext::new(&WF, &log)).unwrap_err();
    assert!(matches!(err, PreconditionError::ApprovalAlreadyRecorded { .. }));

    // Different approver lands fine.
    let second: EventBody<Wf> = EventBody::ApprovalRecorded {
        target: unit_created_id,
        approver: Person(CompactString::from("bob")),
        decision: Decision::Approved,
        rationale: Rationale::new("bob also agrees").unwrap(),
    };
    assert!(second.check_precondition(&AppendContext::new(&WF, &log)).is_ok());
}

// --- ToolCallFailed ---------------------------------------------------

fn tool_call_failed(tool: &str, call_id: &str, attempt: u32) -> EventBody<Wf> {
    EventBody::ToolCallFailed {
        tool: tool.into(),
        call_id: call_id.into(),
        attempt: NonZeroU32::new(attempt).expect("attempt is non-zero"),
        reason: knotch_kernel::event::FailureReason::Timeout { after_secs: 5 },
    }
}

#[test]
fn tool_call_failed_accepts_strictly_increasing_attempts_per_pair() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let first = tool_call_failed("Bash", "call-1", 1);
    assert!(first.check_precondition(&ctx(&l)).is_ok());

    let l2 = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        tool_call_failed("Bash", "call-1", 1),
    ]);
    let second = tool_call_failed("Bash", "call-1", 2);
    assert!(second.check_precondition(&ctx(&l2)).is_ok());
}

#[test]
fn tool_call_failed_rejects_non_monotonic_attempt_on_same_pair() {
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        tool_call_failed("Bash", "call-1", 3),
    ]);
    let regression = tool_call_failed("Bash", "call-1", 2);
    let err = regression.check_precondition(&ctx(&l)).unwrap_err();
    assert!(
        matches!(err, PreconditionError::NonMonotonicAttempt { attempt: 2, prior: 3 }),
        "got {err:?}",
    );
    // Equal attempt is also rejected — strict monotonicity.
    let equal = tool_call_failed("Bash", "call-1", 3);
    let err = equal.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NonMonotonicAttempt { attempt: 3, prior: 3 }));
}

#[test]
fn tool_call_failed_attempts_are_scoped_per_tool_and_call_id_pair() {
    // A prior ToolCallFailed on (Bash, call-1) does not constrain
    // attempts on (Bash, call-2) or (Read, call-1).
    let l = log(vec![
        EventBody::UnitCreated { scope: Scope::Standard },
        tool_call_failed("Bash", "call-1", 5),
    ]);
    let different_call_id = tool_call_failed("Bash", "call-2", 1);
    assert!(different_call_id.check_precondition(&ctx(&l)).is_ok());
    let different_tool = tool_call_failed("Read", "call-1", 1);
    assert!(different_tool.check_precondition(&ctx(&l)).is_ok());
}

// --- ModelSwitched ----------------------------------------------------

fn model_switched(from: &str, to: &str) -> EventBody<Wf> {
    EventBody::ModelSwitched {
        from: knotch_kernel::causation::ModelId(from.into()),
        to: knotch_kernel::causation::ModelId(to.into()),
    }
}

#[test]
fn model_switched_accepts_distinct_models() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body = model_switched("sonnet-4-6", "opus-4-7");
    assert!(body.check_precondition(&ctx(&l)).is_ok());
}

#[test]
fn model_switched_rejects_noop_switch() {
    let l = log(vec![EventBody::UnitCreated { scope: Scope::Standard }]);
    let body = model_switched("opus-4-7", "opus-4-7");
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NoOpModelSwitch { .. }), "got {err:?}");
}

// --- supersede-awareness regressions (C1 audit) ----------------------

#[test]
fn reconcile_recovered_rejected_when_prior_failure_was_superseded() {
    let anchor = RetryAnchor::Observer { name: "obs".into() };
    let failed_id = EventId::new_v7();
    let events: Vec<knotch_kernel::Event<Wf>> = vec![
        knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        },
        knotch_kernel::Event {
            id: failed_id,
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::ReconcileFailed {
                anchor: anchor.clone(),
                attempt: NonZeroU32::new(1).unwrap(),
                kind: FailureKind::ObserverFailed,
            },
            supersedes: None,
        },
        knotch_kernel::Event {
            id: EventId::new_v7(),
            at: Timestamp::now(),
            causation: causation(),
            extension: (),
            body: EventBody::EventSuperseded {
                target: failed_id,
                reason: Rationale::new("retract failure").unwrap(),
            },
            supersedes: None,
        },
    ];
    let l = Log::from_events(UnitId::try_new("u").unwrap(), events);
    let body: EventBody<Wf> = EventBody::ReconcileRecovered {
        anchor,
        attempts_total: NonZeroU32::new(2).unwrap(),
    };
    let err = body.check_precondition(&ctx(&l)).unwrap_err();
    assert!(matches!(err, PreconditionError::NoPriorFailure), "got {err:?}");
}

#[test]
fn approval_rejected_when_target_was_superseded() {
    use compact_str::CompactString;
    use knotch_kernel::causation::Person;
    let unit_created_id = EventId::new_v7();
    let events: Vec<knotch_kernel::Event<Wf>> = vec![
        knotch_kernel::Event {
            id: unit_created_id,
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
            body: EventBody::EventSuperseded {
                target: unit_created_id,
                reason: Rationale::new("retract unit").unwrap(),
            },
            supersedes: None,
        },
    ];
    let l = Log::from_events(UnitId::try_new("u").unwrap(), events);
    let body: EventBody<Wf> = EventBody::ApprovalRecorded {
        target: unit_created_id,
        approver: Person(CompactString::from("alice")),
        decision: Decision::Approved,
        rationale: Rationale::new("approving retracted event").unwrap(),
    };
    let err = body.check_precondition(&AppendContext::new(&WF, &l)).unwrap_err();
    assert!(matches!(err, PreconditionError::ApprovalTargetMissing(_)), "got {err:?}");
}
