//! File-based advisory lock with an in-process serializer.
//!
//! The adapter layers two synchronization primitives:
//!
//! 1. A per-process async `Mutex` keyed by `UnitId`, ensuring that concurrent tasks
//!    inside a single process never race. Required because POSIX fcntl advisory locks are
//!    per-process, not per-file-descriptor — opening a second fd inside the same process
//!    would inherit the lock rather than contend with it.
//! 2. A cross-process advisory lock via `fs4`, preventing two separate processes from
//!    writing at once.
//!
//! A held `LockGuard` owns both the in-process mutex permit (as a
//! `OwnedMutexGuard`) and the locked file handle. Dropping the guard
//! drops the file (releasing the fs4 lock) and then releases the
//! mutex permit.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use fs4::tokio::AsyncFileExt;
use jiff::Timestamp;
use knotch_kernel::UnitId;
use tokio::{
    fs::File,
    io::AsyncWriteExt as _,
    sync::{Mutex as AsyncMutex, OwnedMutexGuard},
};

use crate::{
    Lock,
    error::LockError,
    metadata::{LockMetadata, LockOwner},
};

type UnitMutexes = DashMap<String, Arc<AsyncMutex<()>>>;

/// File-based lock adapter rooted at a base directory.
#[derive(Debug, Clone)]
pub struct FileLock {
    root: PathBuf,
    poll_interval: Duration,
    mutexes: Arc<UnitMutexes>,
}

impl FileLock {
    /// Construct a lock adapter rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            poll_interval: Duration::from_millis(50),
            mutexes: Arc::new(DashMap::new()),
        }
    }

    /// Override the contention-poll interval (default: 50 ms).
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    fn lock_path(&self, unit: &UnitId) -> PathBuf {
        self.root.join(unit.as_str()).join(".lock")
    }

    fn meta_path(&self, unit: &UnitId) -> PathBuf {
        self.root.join(unit.as_str()).join(".lock.meta")
    }

    fn mutex_for(&self, unit: &UnitId) -> Arc<AsyncMutex<()>> {
        self.mutexes
            .entry(unit.as_str().to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }
}

impl Lock for FileLock {
    async fn acquire(
        &self,
        unit: &UnitId,
        timeout: Duration,
        lease: Duration,
    ) -> Result<LockGuard, LockError> {
        let start = Instant::now();
        let lock_path = self.lock_path(unit);
        let meta_path = self.meta_path(unit);

        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| LockError::io(parent, e))?;
        }

        // 1. In-process serialization — wait for our turn among concurrent tasks in this process.
        let mutex = self.mutex_for(unit);
        let permit = match tokio::time::timeout(timeout, mutex.lock_owned()).await {
            Ok(p) => p,
            Err(_) => {
                return Err(LockError::Timeout {
                    unit: unit.as_str().to_owned(),
                    waited: start.elapsed(),
                });
            }
        };

        // 2. Cross-process fs4 advisory lock with reclaim loop.
        loop {
            let file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .await
                .map_err(|e| LockError::io(&lock_path, e))?;

            match file.try_lock_exclusive() {
                Ok(()) => {
                    let was_reclaimed = check_reclaimed_marker(&meta_path).await;
                    let owner = LockOwner::current();
                    let meta = LockMetadata { owner, acquired_at: Timestamp::now(), lease };
                    write_metadata(&meta_path, &meta).await?;
                    return Ok(LockGuard {
                        file: Some(file),
                        _permit: permit,
                        unit: unit.clone(),
                        was_reclaimed,
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    // Cross-process contention — probe prior holder.
                }
                Err(err) => return Err(LockError::io(&lock_path, err)),
            }

            let prior = read_metadata(&meta_path).await?;
            if should_reclaim(&prior) {
                drop(file);
                let _ = tokio::fs::remove_file(&lock_path).await;
                mark_prior_for_reclaim(&meta_path).await?;
                continue;
            }

            if start.elapsed() >= timeout {
                return Err(LockError::Timeout {
                    unit: unit.as_str().to_owned(),
                    waited: start.elapsed(),
                });
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

async fn read_metadata(path: &Path) -> Result<Option<LockMetadata>, LockError> {
    match tokio::fs::read(path).await {
        Ok(bytes) if bytes.is_empty() => Ok(None),
        Ok(bytes) => match serde_json::from_slice::<LockMetadata>(&bytes) {
            Ok(meta) => Ok(Some(meta)),
            Err(_) => Ok(None), // malformed — treat as no prior holder info
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(LockError::io(path, err)),
    }
}

async fn write_metadata(path: &Path, meta: &LockMetadata) -> Result<(), LockError> {
    let bytes = serde_json::to_vec_pretty(meta)
        .map_err(|e| LockError::MalformedMetadata { path: path.to_owned(), source: e })?;
    let mut file = File::create(path).await.map_err(|e| LockError::io(path, e))?;
    file.write_all(&bytes).await.map_err(|e| LockError::io(path, e))?;
    file.flush().await.map_err(|e| LockError::io(path, e))?;
    file.sync_all().await.map_err(|e| LockError::io(path, e))?;
    Ok(())
}

fn should_reclaim(meta: &Option<LockMetadata>) -> bool {
    let Some(meta) = meta else { return false };
    if meta.is_expired(Timestamp::now()) {
        return true;
    }
    !pid_alive(meta.owner.pid)
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    use rustix::process::{Pid, test_kill_process};
    let Some(pid) = Pid::from_raw(pid as i32) else { return false };
    test_kill_process(pid).is_ok()
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

async fn mark_prior_for_reclaim(path: &Path) -> Result<(), LockError> {
    let marker = serde_json::json!({ "__knotch_reclaim__": true });
    let bytes = serde_json::to_vec(&marker).unwrap_or_default();
    let _ = tokio::fs::write(path, bytes).await;
    Ok(())
}

async fn check_reclaimed_marker(path: &Path) -> bool {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("__knotch_reclaim__").and_then(serde_json::Value::as_bool))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// RAII handle returned by `Lock::acquire`. Releases the fs4 lock
/// and the in-process mutex on drop.
#[derive(Debug)]
pub struct LockGuard {
    file: Option<File>,
    _permit: OwnedMutexGuard<()>,
    unit: UnitId,
    was_reclaimed: bool,
}

impl LockGuard {
    /// Was this lock obtained by reclaiming a stale prior holder?
    #[must_use]
    pub fn was_reclaimed(&self) -> bool {
        self.was_reclaimed
    }

    /// Unit the lock is held for.
    #[must_use]
    pub fn unit(&self) -> &UnitId {
        &self.unit
    }

    /// Explicitly release the lock. Equivalent to dropping.
    pub async fn release(self) {
        drop(self);
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Drop the file (releases the fs4 lock). The tokio File's
        // close is scheduled to the blocking pool, which is fine for
        // cross-process semantics — the in-process mutex continues
        // to block same-process contenders until the permit drops
        // on the next line.
        if let Some(file) = self.file.take() {
            drop(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, FileLock) {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = FileLock::new(dir.path()).with_poll_interval(Duration::from_millis(10));
        (dir, lock)
    }

    #[tokio::test]
    async fn acquire_release_roundtrip() {
        let (_dir, lock) = setup();
        let unit = UnitId::try_new("alpha").unwrap();
        let guard = lock
            .acquire(&unit, Duration::from_secs(5), Duration::from_secs(60))
            .await
            .expect("acquire");
        assert_eq!(guard.unit().as_str(), "alpha");
        assert!(!guard.was_reclaimed());
        drop(guard);
    }

    #[tokio::test]
    async fn sequential_acquire_works() {
        let (_dir, lock) = setup();
        let unit = UnitId::try_new("gamma").unwrap();
        for _ in 0..3 {
            let _g = lock
                .acquire(&unit, Duration::from_secs(1), Duration::from_secs(60))
                .await
                .expect("acquire");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_tasks_are_serialized_in_process() {
        let (_dir, lock) = setup();
        let lock = Arc::new(lock);
        let unit = Arc::new(UnitId::try_new("serial").unwrap());
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let lock = lock.clone();
            let unit = unit.clone();
            let counter = counter.clone();
            tasks.push(tokio::spawn(async move {
                let _guard = lock
                    .acquire(&unit, Duration::from_secs(5), Duration::from_secs(60))
                    .await
                    .expect("acquire");
                let prev = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Inside the lock, the counter must be stable.
                tokio::time::sleep(Duration::from_millis(5)).await;
                assert_eq!(
                    counter.load(std::sync::atomic::Ordering::SeqCst),
                    prev + 1,
                    "counter mutated by another task inside the lock"
                );
            }));
        }
        for t in tasks {
            t.await.expect("join");
        }
    }
}
