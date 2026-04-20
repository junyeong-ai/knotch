//! Integration tests for `knotch_agent::queue::post_tool_append`.
//!
//! Exercises the PostToolUse-shaped contract from
//! `.claude/rules/hook-integration.md`: retry on transient failures,
//! enqueue on retry exhaustion, orphan-log on queue full. A thin
//! `FaultyRepo` wrapper around `InMemoryRepository` injects the
//! append failures; the rest of the Repository surface delegates.

#![allow(missing_docs)]

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use knotch_agent::{
    HookOutput,
    queue::{OverflowPolicy, PostToolContext, QueueConfig, post_tool_append, queue_size},
};
use knotch_kernel::{
    AppendMode, AppendReport, Causation, CommitKind, CommitRef, CommitStatus, EventBody, Log,
    Proposal, Repository, RepositoryError, Scope, UnitId,
    causation::{Principal, Source, Trigger},
    event::{SubscribeEvent, SubscribeMode},
    repository::{CacheMutator, PinStream},
    time::Timestamp,
};
use knotch_testing::InMemoryRepository;
use knotch_workflow::Knotch;

/// Wraps an `InMemoryRepository` and forces `append` to return
/// `RepositoryError::Storage(...)` for the first `fail_count`
/// attempts, then delegates. Used to exercise the retry + enqueue
/// paths in `post_tool_append`.
#[derive(Clone)]
struct FaultyRepo {
    inner: Arc<InMemoryRepository<Knotch>>,
    attempts: Arc<AtomicU32>,
    fail_count: u32,
}

impl FaultyRepo {
    fn new(fail_count: u32) -> Self {
        Self {
            inner: Arc::new(InMemoryRepository::<Knotch>::new(Knotch)),
            attempts: Arc::new(AtomicU32::new(0)),
            fail_count,
        }
    }

    fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::SeqCst)
    }

    fn transient_storage_error() -> RepositoryError {
        RepositoryError::Storage(Box::new(std::io::Error::other("injected transient failure")))
    }
}

impl Repository<Knotch> for FaultyRepo {
    fn workflow(&self) -> &Knotch {
        self.inner.workflow()
    }

    async fn append(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<Knotch>>,
        mode: AppendMode,
    ) -> Result<AppendReport<Knotch>, RepositoryError> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_count {
            return Err(Self::transient_storage_error());
        }
        self.inner.append(unit, proposals, mode).await
    }

    async fn load(&self, unit: &UnitId) -> Result<Arc<Log<Knotch>>, RepositoryError> {
        self.inner.load(unit).await
    }

    async fn load_until(
        &self,
        unit: &UnitId,
        cutoff: Timestamp,
    ) -> Result<Arc<Log<Knotch>>, RepositoryError> {
        self.inner.load_until(unit, cutoff).await
    }

    async fn subscribe(
        &self,
        unit: &UnitId,
        mode: SubscribeMode,
    ) -> Result<PinStream<SubscribeEvent<Knotch>>, RepositoryError> {
        self.inner.subscribe(unit, mode).await
    }

    fn list_units(&self) -> PinStream<Result<UnitId, RepositoryError>> {
        <InMemoryRepository<Knotch> as Repository<Knotch>>::list_units(&self.inner)
    }

    async fn with_cache(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<Knotch>>,
        mode: AppendMode,
        mutate_cache: CacheMutator,
    ) -> Result<AppendReport<Knotch>, RepositoryError> {
        self.inner.with_cache(unit, proposals, mode, mutate_cache).await
    }
}

fn causation() -> Causation {
    Causation::new(
        Source::Hook,
        Principal::System { service: "test".into() },
        Trigger::GitHook { name: "post-tool".into() },
    )
}

fn milestone_proposal(id: &str, sha: &str) -> Proposal<Knotch> {
    Proposal {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::MilestoneShipped {
            milestone: knotch_workflow::TaskId(id.into()),
            commit: CommitRef::new(sha),
            commit_kind: CommitKind::Feat,
            status: CommitStatus::Verified,
        },
        supersedes: None,
    }
}

async fn seed_unit<R: Repository<Knotch>>(repo: &R, unit: &UnitId) {
    let proposal = Proposal {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::BestEffort).await.expect("seed");
}

fn make_ctx<'a>(
    queue_dir: &'a std::path::Path,
    queue_config: &'a QueueConfig,
    home: &'a std::path::Path,
    cwd: &'a std::path::Path,
) -> PostToolContext<'a> {
    PostToolContext { queue_dir, queue_config, home, cwd, hook_name: "test-hook" }
}

#[tokio::test]
async fn happy_path_single_append_no_queue() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue_dir = tmp.path().join(".knotch/queue");
    let home = tmp.path().join("home");
    let cwd = tmp.path().to_path_buf();
    let cfg = QueueConfig::default();
    let ctx = make_ctx(&queue_dir, &cfg, &home, &cwd);

    let repo = FaultyRepo::new(0);
    let unit = UnitId::new("happy");
    seed_unit(&repo, &unit).await;

    let proposal = milestone_proposal("ms-1", "abc1234");
    let out = post_tool_append::<Knotch, _>(&repo, &unit, proposal, ctx).await.expect("append");

    assert_eq!(out, HookOutput::Continue);
    // `attempts` counts `seed_unit` (1) + first `post_tool_append` (1).
    assert_eq!(repo.attempts(), 2);
    assert_eq!(queue_size(&queue_dir).unwrap(), 0, "happy path never queues");
}

#[tokio::test]
async fn retry_exhausted_enqueues_for_reconcile() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue_dir = tmp.path().join(".knotch/queue");
    let home = tmp.path().join("home");
    let cwd = tmp.path().to_path_buf();
    let cfg = QueueConfig::default();
    let ctx = make_ctx(&queue_dir, &cfg, &home, &cwd);

    // Fail 999 times — well beyond `POST_TOOL_MAX_ATTEMPTS=3`, so every
    // retry surfaces the transient error. The helper must then queue.
    let repo = FaultyRepo::new(999);
    let unit = UnitId::new("queued");

    let proposal = milestone_proposal("ms-2", "def5678");
    let out = post_tool_append::<Knotch, _>(&repo, &unit, proposal, ctx).await.expect("queued");

    assert_eq!(out, HookOutput::Continue, "terminal path always Continue");
    assert_eq!(repo.attempts(), 3, "3× attempts then bail to queue");
    assert_eq!(queue_size(&queue_dir).unwrap(), 1, "one queue entry persisted");

    // Orphan log should be untouched — queue had room.
    let orphan_path = home.join(".knotch/orphan.log");
    assert!(!orphan_path.exists(), "orphan log must not be touched when queue succeeds");
}

#[tokio::test]
async fn queue_full_falls_back_to_orphan_log() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue_dir = tmp.path().join(".knotch/queue");
    let home = tmp.path().join("home");
    let cwd = tmp.path().to_path_buf();
    // Cap the queue at a single entry and pre-fill it so the next
    // enqueue hits `HookError::QueueFull`.
    let cfg = QueueConfig { max_entries: 1, overflow: OverflowPolicy::Reject };
    let ctx = make_ctx(&queue_dir, &cfg, &home, &cwd);

    std::fs::create_dir_all(&queue_dir).unwrap();
    std::fs::write(queue_dir.join("aaaaaaa.json"), b"{}").expect("pre-fill");
    assert_eq!(queue_size(&queue_dir).unwrap(), 1);

    let repo = FaultyRepo::new(999);
    let unit = UnitId::new("orphaned");

    let proposal = milestone_proposal("ms-3", "deadbee");
    let out = post_tool_append::<Knotch, _>(&repo, &unit, proposal, ctx).await.expect("orphaned");

    assert_eq!(out, HookOutput::Continue, "queue-full never blocks the PostToolUse exit");
    // Queue size is unchanged — Reject policy refused the new entry.
    assert_eq!(queue_size(&queue_dir).unwrap(), 1);

    // Orphan log must now carry a record.
    let orphan_path = home.join(".knotch/orphan.log");
    assert!(orphan_path.exists(), "orphan log was written on QueueFull fallback");
    let body = std::fs::read_to_string(&orphan_path).expect("read orphan");
    assert!(body.contains("test-hook"), "orphan record tags the hook: {body}");
    assert!(body.contains("queue-full"), "orphan record explains the fallback: {body}");
}

#[tokio::test]
async fn best_effort_rejection_continues_without_retry_or_queue() {
    // PostToolUse uses AppendMode::BestEffort: precondition failures
    // surface as `AppendReport::rejected` rather than top-level Err,
    // so the repository returns Ok. `post_tool_append` must honor
    // that — no retry, no queue, just Continue — because a permanent
    // rejection (e.g. "milestone already shipped") is informational
    // for the hook, not a signal to retry.
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue_dir = tmp.path().join(".knotch/queue");
    let home = tmp.path().join("home");
    let cwd = tmp.path().to_path_buf();
    let cfg = QueueConfig::default();
    let ctx = make_ctx(&queue_dir, &cfg, &home, &cwd);

    let repo = FaultyRepo::new(0);
    let unit = UnitId::new("bestfx");
    seed_unit(&repo, &unit).await; // first UnitCreated lands
    let before_attempts = repo.attempts();

    let duplicate = Proposal::<Knotch> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    let out = post_tool_append::<Knotch, _>(&repo, &unit, duplicate, ctx)
        .await
        .expect("BestEffort rejection returns Ok at the helper boundary");
    assert_eq!(out, HookOutput::Continue);
    assert_eq!(
        repo.attempts() - before_attempts,
        1,
        "rejection from BestEffort is not a transient error — must not retry",
    );
    assert_eq!(queue_size(&queue_dir).unwrap(), 0, "nothing to queue when repo reports Ok");
}

#[tokio::test]
async fn retry_eventually_succeeds_on_transient_recovery() {
    // Fails exactly twice (attempts 0, 1) then succeeds on attempt 2.
    // Verifies the exponential backoff + retry actually recovers —
    // not just that the queue catches it.
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue_dir = tmp.path().join(".knotch/queue");
    let home = tmp.path().join("home");
    let cwd = tmp.path().to_path_buf();
    let cfg = QueueConfig::default();
    let ctx = make_ctx(&queue_dir, &cfg, &home, &cwd);

    let repo = FaultyRepo::new(2);
    let unit = UnitId::new("recovers");
    // `seed_unit` uses up attempt 0 (succeeds since fail_count=2 but…)
    // Wait — FaultyRepo fails the first `fail_count` attempts. For a
    // clean test we seed via `inner` directly so the seed does not
    // eat into the attempt budget.
    let proposal_seed = Proposal::<Knotch> {
        causation: causation(),
        extension: Default::default(),
        body: EventBody::UnitCreated { scope: Scope::Standard },
        supersedes: None,
    };
    repo.inner.append(&unit, vec![proposal_seed], AppendMode::BestEffort).await.expect("seed");

    let proposal = milestone_proposal("ms-4", "cafe001");
    let out = post_tool_append::<Knotch, _>(&repo, &unit, proposal, ctx).await.expect("recovered");
    assert_eq!(out, HookOutput::Continue);
    assert_eq!(repo.attempts(), 3, "two failures + one success");
    assert_eq!(queue_size(&queue_dir).unwrap(), 0, "recovered — nothing queued");
}

/// Wire check — the `PostToolContext` struct path is exported and
/// constructible, guarding against accidental signature drift.
#[allow(dead_code)]
fn ctx_compiles_from_public_api() {
    let queue_dir: PathBuf = PathBuf::from("/tmp/q");
    let cfg = QueueConfig::default();
    let home = PathBuf::from("/tmp/home");
    let cwd = PathBuf::from("/tmp/cwd");
    let _ = PostToolContext {
        queue_dir: &queue_dir,
        queue_config: &cfg,
        home: &home,
        cwd: &cwd,
        hook_name: "probe",
    };
}
