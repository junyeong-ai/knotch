//! Integration tests for concurrent append behavior.
//!
//! These tests exercise the Storage + Lock pairing end-to-end.
//! The kernel Repository trait is not used directly here because its
//! implementation lands in Phase 5; instead we simulate the
//! "acquire → load → append → release" flow that a FileRepository
//! will use.

use std::{sync::Arc, time::Duration};

use knotch_kernel::UnitId;
use knotch_lock::{FileLock, Lock};
use knotch_storage::{FileSystemStorage, Storage};

async fn append_one(
    storage: &FileSystemStorage,
    lock: &FileLock,
    unit: &UnitId,
    line: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let guard = lock.acquire(unit, Duration::from_secs(10), Duration::from_secs(60)).await?;
    let (lines, _report) = storage.load(unit).await?;
    let expected_len = lines.len() as u64;
    storage.append(unit, expected_len, vec![line]).await?;
    drop(guard);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn sixteen_threads_each_appending_sixty_four_events_converge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Arc::new(FileSystemStorage::new(dir.path()));
    let lock = Arc::new(FileLock::new(dir.path()).with_poll_interval(Duration::from_millis(5)));
    let unit = Arc::new(UnitId::try_new("convergence").unwrap());

    let mut tasks = Vec::new();
    const WORKERS: usize = 16;
    const PER_WORKER: usize = 64;

    for w in 0..WORKERS {
        let storage = storage.clone();
        let lock = lock.clone();
        let unit = unit.clone();
        tasks.push(tokio::spawn(async move {
            for i in 0..PER_WORKER {
                let line = format!(r#"{{"worker":{w},"seq":{i}}}"#);
                append_one(&storage, &lock, &unit, line).await.expect("append");
            }
        }));
    }

    for t in tasks {
        t.await.expect("join");
    }

    let (lines, report) = storage.load(&unit).await.expect("load");
    assert!(report.is_clean(), "report should be clean: {report:?}");
    assert_eq!(
        lines.len(),
        WORKERS * PER_WORKER,
        "expected {} lines, got {}",
        WORKERS * PER_WORKER,
        lines.len(),
    );

    // Every worker/seq combination must appear exactly once — the
    // single-writer invariant under the lock serializes appends.
    let mut seen = std::collections::HashSet::new();
    for line in &lines {
        assert!(seen.insert(line.clone()), "duplicate line observed: {line}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contending_writers_never_corrupt_log() {
    // Four workers hammer the same unit from different tasks; after
    // the test, every line must parse as JSON.
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Arc::new(FileSystemStorage::new(dir.path()));
    let lock = Arc::new(FileLock::new(dir.path()).with_poll_interval(Duration::from_millis(5)));
    let unit = Arc::new(UnitId::try_new("no-corruption").unwrap());

    let mut tasks = Vec::new();
    for w in 0..4 {
        let storage = storage.clone();
        let lock = lock.clone();
        let unit = unit.clone();
        tasks.push(tokio::spawn(async move {
            for i in 0..16 {
                let line = format!(r#"{{"worker":{w},"seq":{i},"len":{}}}"#, w * 1000 + i);
                append_one(&storage, &lock, &unit, line).await.expect("append");
            }
        }));
    }

    for t in tasks {
        t.await.expect("join");
    }

    let (lines, report) = storage.load(&unit).await.expect("load");
    assert!(report.is_clean());
    assert_eq!(lines.len(), 4 * 16);
    for line in &lines {
        let _value: serde_json::Value =
            serde_json::from_str(line).expect("line must parse as JSON");
    }
}
