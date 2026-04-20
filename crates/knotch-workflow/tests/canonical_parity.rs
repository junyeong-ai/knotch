//! `ConfigWorkflow::canonical()` must behave identically to the typed
//! `Knotch` impl on every observable axis — required phases per scope,
//! terminal-status membership, rationale floor, known statuses,
//! gate prerequisite graph. This locks the "canonical" contract so
//! `knotch init` stamping config TOML never drifts from the typed
//! reference impl.

use knotch_kernel::{
    Causation, Proposal, Rationale, Scope, StatusId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, EventBody},
    fingerprint_proposal,
};
use knotch_workflow::{ConfigWorkflow, DynamicPhase, Knotch, KnotchGate, KnotchPhase, TaskId};

#[test]
fn required_phases_match_on_standard_scope() {
    let config = ConfigWorkflow::canonical();
    let typed = Knotch;
    let c = config.required_phases(&Scope::Standard);
    let t = typed.required_phases(&Scope::Standard);
    assert_eq!(c.len(), t.len());
    for (c_ph, t_ph) in c.iter().zip(t.iter()) {
        assert_eq!(c_ph.0, knotch_kernel::PhaseKind::id(t_ph).as_ref(), "phase ids diverge",);
    }
}

#[test]
fn required_phases_match_on_tiny_scope() {
    let config = ConfigWorkflow::canonical();
    let typed = Knotch;
    let c = config.required_phases(&Scope::Tiny);
    let t = typed.required_phases(&Scope::Tiny);
    assert_eq!(c.len(), t.len());
    for (c_ph, t_ph) in c.iter().zip(t.iter()) {
        assert_eq!(c_ph.0, knotch_kernel::PhaseKind::id(t_ph).as_ref(),);
    }
}

#[test]
fn terminal_status_set_matches() {
    let config = ConfigWorkflow::canonical();
    let typed = Knotch;
    for s in ["archived", "abandoned", "superseded", "deprecated"] {
        let sid = StatusId::new(s);
        assert!(config.is_terminal_status(&sid));
        assert!(typed.is_terminal_status(&sid));
    }
    for s in ["draft", "in_progress", "in_review", "shipped"] {
        let sid = StatusId::new(s);
        assert!(!config.is_terminal_status(&sid));
        assert!(!typed.is_terminal_status(&sid));
    }
}

#[test]
fn known_statuses_match() {
    let config = ConfigWorkflow::canonical();
    let typed = Knotch;
    let c: Vec<String> = config.known_statuses().iter().map(|s| s.as_ref().to_owned()).collect();
    let t: Vec<String> = typed.known_statuses().iter().map(|s| s.as_ref().to_owned()).collect();
    assert_eq!(c, t);
}

#[test]
fn min_rationale_chars_match() {
    assert_eq!(ConfigWorkflow::canonical().min_rationale_chars(), Knotch.min_rationale_chars(),);
}

#[test]
fn gate_prereq_graph_matches() {
    let config = ConfigWorkflow::canonical();
    let cases: [(KnotchGate, &[KnotchGate]); 5] = [
        (KnotchGate::G0Scope, &[]),
        (KnotchGate::G1Clarify, &[KnotchGate::G0Scope]),
        (KnotchGate::G2Plan, &[KnotchGate::G0Scope, KnotchGate::G1Clarify]),
        (KnotchGate::G3Review, &[KnotchGate::G0Scope, KnotchGate::G1Clarify, KnotchGate::G2Plan]),
        (
            KnotchGate::G4Drift,
            &[KnotchGate::G0Scope, KnotchGate::G1Clarify, KnotchGate::G2Plan, KnotchGate::G3Review],
        ),
    ];
    for (gate, typed_prereqs) in cases {
        let typed_ids: Vec<String> =
            typed_prereqs.iter().map(|g| knotch_kernel::GateKind::id(g).into_owned()).collect();
        let gate_id = knotch_kernel::GateKind::id(&gate).into_owned();
        let dyn_gate = config.gate(&gate_id).expect("gate in config");
        let via_trait = config.prerequisites_for(dyn_gate);
        let config_ids: Vec<String> = via_trait.iter().map(|g| g.0.to_string()).collect();
        assert_eq!(config_ids, typed_ids, "prereqs diverge for {gate_id}",);
    }
}

#[test]
fn fingerprint_salts_are_disjoint_between_canonical_and_named_config() {
    let canonical = ConfigWorkflow::canonical();
    let canonical_salt = canonical.fingerprint_salt().into_owned();
    assert_eq!(canonical_salt, b"knotch");

    let mut spec = canonical.spec().clone();
    spec.name = "grove".into();
    let adopter = ConfigWorkflow::from_spec(spec).expect("adopter spec validates");
    let adopter_salt = adopter.fingerprint_salt().into_owned();
    assert_eq!(adopter_salt, b"grove");
    assert_ne!(canonical_salt, adopter_salt);
}

/// The critical wire-level guarantee: typed `Knotch` and canonical
/// `ConfigWorkflow` produce **byte-identical fingerprints** for
/// equivalent proposals. This is what lets an adopter switch
/// between the typed path (their own binary) and the config path
/// (the shipped `knotch` CLI) without fingerprints diverging and
/// dedup silently breaking.
#[test]
fn fingerprint_bit_identical_between_typed_and_config_canonical() {
    fn causation() -> Causation {
        Causation::new(
            Source::Cli,
            Principal::System { service: "parity".into() },
            Trigger::Command { name: "test".into() },
        )
    }

    // --- UnitCreated ---
    let typed_unit = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    let config_unit = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_unit).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_unit).unwrap();
    assert_eq!(fp_typed, fp_config, "UnitCreated fingerprint diverges");

    // --- PhaseCompleted ---
    let typed_phase = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::PhaseCompleted {
            phase: KnotchPhase::Specify,
            artifacts: ArtifactList::default(),
        },
        supersedes: None,
    };
    let config_phase = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::PhaseCompleted {
            phase: DynamicPhase::from("specify"),
            artifacts: ArtifactList::default(),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_phase).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_phase).unwrap();
    assert_eq!(fp_typed, fp_config, "PhaseCompleted fingerprint diverges");

    // --- GateRecorded ---
    use knotch_kernel::Decision;
    let typed_gate = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::GateRecorded {
            gate: KnotchGate::G0Scope,
            decision: Decision::Approved,
            rationale: Rationale::new("scope locked at standard").unwrap(),
        },
        supersedes: None,
    };
    let config_gate = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::GateRecorded {
            gate: knotch_workflow::DynamicGate("g0-scope".into()),
            decision: Decision::Approved,
            rationale: Rationale::new("scope locked at standard").unwrap(),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_gate).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_gate).unwrap();
    assert_eq!(fp_typed, fp_config, "GateRecorded fingerprint diverges");

    // --- MilestoneShipped ---
    use knotch_kernel::{
        CommitStatus,
        event::{CommitKind, CommitRef},
    };
    let typed_ms = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::MilestoneShipped {
            milestone: TaskId("ship-auth".into()),
            commit: CommitRef::new("abc1234"),
            commit_kind: CommitKind::Feat,
            status: CommitStatus::Verified,
        },
        supersedes: None,
    };
    let config_ms = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::MilestoneShipped {
            milestone: knotch_workflow::DynamicMilestone("ship-auth".into()),
            commit: CommitRef::new("abc1234"),
            commit_kind: CommitKind::Feat,
            status: CommitStatus::Verified,
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_ms).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_ms).unwrap();
    assert_eq!(fp_typed, fp_config, "MilestoneShipped fingerprint diverges");

    // --- PhaseSkipped ---
    use knotch_kernel::event::SkipKind;
    let typed_skip = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::PhaseSkipped {
            phase: KnotchPhase::Plan,
            reason: SkipKind::ScopeTooNarrow,
        },
        supersedes: None,
    };
    let config_skip = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::PhaseSkipped {
            phase: DynamicPhase::from("plan"),
            reason: SkipKind::ScopeTooNarrow,
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_skip).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_skip).unwrap();
    assert_eq!(fp_typed, fp_config, "PhaseSkipped fingerprint diverges");

    // --- MilestoneReverted ---
    let typed_rev = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::MilestoneReverted {
            milestone: TaskId("ship-auth".into()),
            original: CommitRef::new("abc1234"),
            revert: CommitRef::new("def5678"),
        },
        supersedes: None,
    };
    let config_rev = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::MilestoneReverted {
            milestone: knotch_workflow::DynamicMilestone::from("ship-auth"),
            original: CommitRef::new("abc1234"),
            revert: CommitRef::new("def5678"),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_rev).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_rev).unwrap();
    assert_eq!(fp_typed, fp_config, "MilestoneReverted fingerprint diverges");

    // --- MilestoneVerified ---
    let typed_ver = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::MilestoneVerified {
            milestone: TaskId("ship-auth".into()),
            commit: CommitRef::new("abc1234"),
        },
        supersedes: None,
    };
    let config_ver = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::MilestoneVerified {
            milestone: knotch_workflow::DynamicMilestone::from("ship-auth"),
            commit: CommitRef::new("abc1234"),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_ver).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_ver).unwrap();
    assert_eq!(fp_typed, fp_config, "MilestoneVerified fingerprint diverges");

    // --- StatusTransitioned ---
    let typed_st = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::StatusTransitioned {
            target: StatusId::new("in_review"),
            forced: false,
            rationale: None,
        },
        supersedes: None,
    };
    let config_st = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::StatusTransitioned {
            target: StatusId::new("in_review"),
            forced: false,
            rationale: None,
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_st).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_st).unwrap();
    assert_eq!(fp_typed, fp_config, "StatusTransitioned fingerprint diverges");

    // --- ReconcileFailed ---
    use std::num::NonZeroU32;

    use knotch_kernel::event::{ReconcileFailureKind, RetryAnchor};
    let anchor = RetryAnchor::Observer { name: "git-log".into() };
    let typed_rf = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::ReconcileFailed {
            anchor: anchor.clone(),
            kind: ReconcileFailureKind::ObserverFailed,
            attempt: NonZeroU32::new(1).unwrap(),
        },
        supersedes: None,
    };
    let config_rf = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::ReconcileFailed {
            anchor: anchor.clone(),
            kind: ReconcileFailureKind::ObserverFailed,
            attempt: NonZeroU32::new(1).unwrap(),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_rf).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_rf).unwrap();
    assert_eq!(fp_typed, fp_config, "ReconcileFailed fingerprint diverges");

    // --- ReconcileRecovered ---
    let typed_rr = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::ReconcileRecovered {
            anchor: anchor.clone(),
            attempts_total: NonZeroU32::new(2).unwrap(),
        },
        supersedes: None,
    };
    let config_rr = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::ReconcileRecovered { anchor, attempts_total: NonZeroU32::new(2).unwrap() },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_rr).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_rr).unwrap();
    assert_eq!(fp_typed, fp_config, "ReconcileRecovered fingerprint diverges");

    // --- EventSuperseded ---
    use knotch_kernel::EventId;
    let target_id = EventId::new_v7();
    let typed_sup = Proposal::<Knotch> {
        causation: causation(),
        extension: (),
        body: EventBody::EventSuperseded {
            target: target_id,
            reason: Rationale::new("correcting earlier miscoding").unwrap(),
        },
        supersedes: None,
    };
    let config_sup = Proposal::<ConfigWorkflow> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::EventSuperseded {
            target: target_id,
            reason: Rationale::new("correcting earlier miscoding").unwrap(),
        },
        supersedes: None,
    };
    let fp_typed = fingerprint_proposal(&Knotch, &typed_sup).unwrap();
    let fp_config = fingerprint_proposal(&ConfigWorkflow::canonical(), &config_sup).unwrap();
    assert_eq!(fp_typed, fp_config, "EventSuperseded fingerprint diverges");
}

#[test]
fn parse_phase_resolves_each_canonical_id() {
    let config = ConfigWorkflow::canonical();
    for typed in [
        KnotchPhase::Specify,
        KnotchPhase::Plan,
        KnotchPhase::Build,
        KnotchPhase::Review,
        KnotchPhase::Ship,
    ] {
        let id = knotch_kernel::PhaseKind::id(&typed).into_owned();
        let parsed = config.parse_phase(&id).unwrap_or_else(|| {
            panic!("config canonical must parse `{id}`");
        });
        assert_eq!(parsed.0.as_str(), id.as_str());
    }
}
