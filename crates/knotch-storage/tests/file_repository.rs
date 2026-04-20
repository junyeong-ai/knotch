//! End-to-end FileRepository<W> tests.

#![allow(missing_docs)]

use std::{borrow::Cow, sync::Arc};

use knotch_derive::MilestoneKind;
use knotch_kernel::{
    AppendMode, Causation, PhaseKind, Proposal, Repository, Scope, UnitId, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{CommitKind, CommitRef, EventBody, SkipKind},
    project::shipped_milestones,
};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum Phase {
    Only,
}

impl PhaseKind for Phase {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("only")
    }
    fn is_skippable(&self, _: &SkipKind) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
enum Milestone {
    Alpha,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Gate {}
impl knotch_kernel::GateKind for Gate {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed("none")
    }
}

#[derive(Debug, Clone, Copy)]
struct Wf;
const PHASES: [Phase; 1] = [Phase::Only];

impl WorkflowKind for Wf {
    type Phase = Phase;
    type Milestone = Milestone;
    type Gate = Gate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("filerepo-test")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
}

fn proposal(body: EventBody<Wf>) -> Proposal<Wf> {
    Proposal {
        causation: Causation::new(
            Source::Cli,
            Principal::System { service: "test".into() },
            Trigger::Manual,
        ),
        extension: (),
        body,
        supersedes: None,
    }
}

#[tokio::test]
async fn append_then_load_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("roundtrip");

    let proposals = vec![
        proposal(EventBody::UnitCreated { scope: Scope::Standard }),
        proposal(EventBody::MilestoneShipped {
            milestone: Milestone::Alpha,
            commit: CommitRef::new("abc1234"),
            commit_kind: CommitKind::Feat,
            status: knotch_kernel::CommitStatus::Verified,
        }),
    ];
    let report = repo.append(&unit, proposals, AppendMode::BestEffort).await.expect("append");
    assert_eq!(report.accepted.len(), 2);
    assert!(report.rejected.is_empty());

    let log = repo.load(&unit).await.expect("load");
    assert_eq!(log.events().len(), 2);
    let shipped = shipped_milestones(&log);
    assert_eq!(shipped, vec![Milestone::Alpha]);
}

#[tokio::test]
async fn replay_on_reopened_repository() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = UnitId::new("reopen");

    {
        let repo = FileRepository::<Wf>::new(dir.path(), Wf);
        repo.append(
            &unit,
            vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
            AppendMode::BestEffort,
        )
        .await
        .expect("append");
    }
    // Fresh instance — proves state survives the process boundary.
    let repo = FileRepository::<Wf>::new(dir.path(), Wf);
    let log = repo.load(&unit).await.expect("load");
    assert_eq!(log.events().len(), 1);
}

#[tokio::test]
async fn duplicate_proposals_are_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Arc::new(FileRepository::<Wf>::new(dir.path(), Wf));
    let unit = UnitId::new("dedup");

    let body = EventBody::MilestoneShipped {
        milestone: Milestone::Beta,
        commit: CommitRef::new("deadbee"),
        commit_kind: CommitKind::Fix,
        status: knotch_kernel::CommitStatus::Verified,
    };
    repo.append(&unit, vec![proposal(body.clone())], AppendMode::BestEffort).await.expect("first");
    let second =
        repo.append(&unit, vec![proposal(body)], AppendMode::BestEffort).await.expect("second");
    assert!(second.accepted.is_empty());
    assert_eq!(second.rejected.len(), 1);
    assert_eq!(second.rejected[0].reason.as_str(), "duplicate");
}

#[tokio::test]
async fn header_written_once_and_schema_version_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = FileRepository::<Wf>::new(dir.path(), Wf);
    let unit = UnitId::new("header");
    repo.append(
        &unit,
        vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .expect("seed");

    let log_path = dir.path().join("header").join("log.jsonl");
    let raw = std::fs::read_to_string(&log_path).expect("read");
    let first_line = raw.lines().next().expect("first line");
    assert!(first_line.contains("\"kind\":\"__header__\""));
    assert!(first_line.contains("\"schema_version\":1"));
    assert!(first_line.contains("\"workflow\":\"filerepo-test\""));
}

// --- P0-5 salt mismatch guard ----------------------------------------------

/// A second workflow that shares `NAME` with `Wf` but overrides the
/// fingerprint salt. Lets us seed a log with `Wf`'s salt and re-open
/// against `WfSaltChanged` to confirm the repository refuses to
/// load / append.
#[derive(Debug, Clone, Copy)]
struct WfSaltChanged;

impl WorkflowKind for WfSaltChanged {
    type Phase = Phase;
    type Milestone = Milestone;
    type Gate = Gate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("filerepo-test")
    }
    fn schema_version(&self) -> u32 {
        1
    }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }
    fn fingerprint_salt(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(b"different-salt")
    }
}

fn proposal_for<W>(body: EventBody<W>) -> Proposal<W>
where
    W: WorkflowKind<Extension = ()>,
{
    Proposal {
        causation: Causation::new(
            Source::Cli,
            Principal::System { service: "test".into() },
            Trigger::Manual,
        ),
        extension: (),
        body,
        supersedes: None,
    }
}

#[tokio::test]
async fn load_rejects_header_with_mismatched_salt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = FileRepository::<Wf>::new(dir.path(), Wf);
    let unit = UnitId::new("salt-drift");
    original
        .append(
            &unit,
            vec![proposal_for::<Wf>(EventBody::UnitCreated { scope: Scope::Standard })],
            AppendMode::BestEffort,
        )
        .await
        .expect("seed with original salt");

    let shifted = FileRepository::<WfSaltChanged>::new(dir.path(), WfSaltChanged);
    let err = shifted.load(&unit).await.expect_err("load must refuse mismatched salt");
    assert!(
        matches!(err, knotch_kernel::RepositoryError::SaltMismatch { .. }),
        "expected SaltMismatch, got {err:?}",
    );
}

#[tokio::test]
async fn append_rejects_header_with_mismatched_salt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = FileRepository::<Wf>::new(dir.path(), Wf);
    let unit = UnitId::new("salt-drift-append");
    original
        .append(
            &unit,
            vec![proposal_for::<Wf>(EventBody::UnitCreated { scope: Scope::Standard })],
            AppendMode::BestEffort,
        )
        .await
        .expect("seed with original salt");

    let shifted = FileRepository::<WfSaltChanged>::new(dir.path(), WfSaltChanged);
    let err = shifted
        .append(
            &unit,
            vec![proposal_for::<WfSaltChanged>(EventBody::UnitCreated { scope: Scope::Standard })],
            AppendMode::BestEffort,
        )
        .await
        .expect_err("append must refuse mismatched salt");
    assert!(
        matches!(err, knotch_kernel::RepositoryError::SaltMismatch { .. }),
        "expected SaltMismatch, got {err:?}",
    );
}

// --- P1-1 load_until (point-in-time snapshot) -----------------------------

#[tokio::test]
async fn load_until_drops_events_after_cutoff() {
    use jiff::{SignedDuration, Timestamp};

    let dir = tempfile::tempdir().expect("tempdir");
    let repo = FileRepository::<Wf>::new(dir.path(), Wf);
    let unit = UnitId::new("timewalk");
    repo.append(
        &unit,
        vec![proposal(EventBody::UnitCreated { scope: Scope::Standard })],
        AppendMode::BestEffort,
    )
    .await
    .expect("seed");
    let full = repo.load(&unit).await.expect("load");
    let first_at = full.events()[0].at;

    // Cutoff one hour BEFORE the first event → empty log.
    let before = first_at.checked_sub(SignedDuration::from_hours(1)).expect("sub");
    let past = repo.load_until(&unit, before).await.expect("load_until");
    assert!(past.events().is_empty());

    // Cutoff AT the first event → event visible.
    let at = repo.load_until(&unit, first_at).await.expect("load_until");
    assert_eq!(at.events().len(), 1);

    // Cutoff in the far future → full history.
    let future = Timestamp::MAX;
    let future_log = repo.load_until(&unit, future).await.expect("load_until");
    assert_eq!(future_log.events().len(), full.events().len());
}
