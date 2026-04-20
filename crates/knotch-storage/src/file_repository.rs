//! File-backed `Repository<W>` — combines `FileSystemStorage` with
//! `knotch_lock::FileLock` and wires kernel invariants
//! (fingerprint dedup, monotonic ordering, atomic cache+event append).
//!
//! The subscribe stream returns empty in v0.1 — real-time file-watch
//! notification lands in Phase 10 hardening.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_stream::try_stream;
use dashmap::DashMap;
use futures::StreamExt as _;
use jiff::Timestamp;
use knotch_kernel::{
    AppendMode, AppendReport, Event, EventId, ExtensionKind as _, Fingerprint, Log, Proposal,
    RepositoryError, UnitId, WorkflowKind,
    event::{RejectedProposal, SubscribeEvent, SubscribeMode},
    fingerprint_event, fingerprint_proposal,
    repository::{CacheMutator, PinStream, Repository, ResumeCache},
    time::{SystemClock, stamp_monotonic},
};
use knotch_lock::{FileLock, Lock};
use knotch_proto::header::Header;
use tokio::sync::broadcast;

use crate::{FileSystemStorage, LoadReport, Storage, StorageError};

/// Default lock acquisition timeout (30 s) and lease (5 min).
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LOCK_LEASE: Duration = Duration::from_secs(300);

/// Capacity of each per-unit broadcast buffer; lagging subscribers
/// see `SubscribeEvent::Lagged` instead of silent drops.
const BROADCAST_CAPACITY: usize = 1024;

/// Maximum attempts for optimistic-CAS retry. With exponential
/// backoff (`CAS_BASE_DELAY * 2^n`) the last attempt waits ~400 ms,
/// giving the interfering writer time to settle before we give up.
const CAS_MAX_RETRIES: u32 = 5;

/// Base delay for the CAS retry backoff. Doubles each attempt.
const CAS_BASE_DELAY: Duration = Duration::from_millis(25);

/// File-backed Repository. Cheap to clone — clones share the
/// underlying `FileSystemStorage`, `FileLock`, and per-unit
/// broadcast channels.
pub struct FileRepository<W: WorkflowKind> {
    workflow: W,
    storage: FileSystemStorage,
    lock: FileLock,
    lock_timeout: Duration,
    lock_lease: Duration,
    /// Per-unit broadcast senders for in-process subscribers.
    /// Cross-process subscription (file-watch) is Phase 10 hardening.
    broadcasters: Arc<DashMap<UnitId, broadcast::Sender<Event<W>>>>,
}

impl<W: WorkflowKind> Clone for FileRepository<W> {
    fn clone(&self) -> Self {
        Self {
            workflow: self.workflow.clone(),
            storage: self.storage.clone(),
            lock: self.lock.clone(),
            lock_timeout: self.lock_timeout,
            lock_lease: self.lock_lease,
            broadcasters: self.broadcasters.clone(),
        }
    }
}

/// Outcome of one `commit_batch` attempt.
enum CommitOutcome<W: WorkflowKind> {
    /// Log append succeeded; fan-out / cache write still pending.
    Committed { accepted: Vec<Event<W>>, rejected: Vec<RejectedProposal<W>> },
    /// `AllOrNothing` mode saw at least one rejection; no events landed.
    Rolled { report: AppendReport<W> },
    /// Optimistic-CAS mismatch — another writer extended the log
    /// between our load and commit. Caller retries.
    CasMismatch { expected: u64, on_disk: u64 },
}

impl<W: WorkflowKind> FileRepository<W> {
    /// Construct a file-backed repository rooted at `root` for the
    /// given `workflow`. The workflow is consulted at append time for
    /// required-phase / terminal-status / rationale-floor decisions,
    /// and also for `fingerprint_salt` on the log header.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, workflow: W) -> Self {
        let root = root.into();
        Self {
            workflow,
            storage: FileSystemStorage::new(root.clone()),
            lock: FileLock::new(root),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            lock_lease: DEFAULT_LOCK_LEASE,
            broadcasters: Arc::new(DashMap::new()),
        }
    }

    /// Borrow the workflow instance this repository was built with.
    #[must_use]
    pub fn workflow(&self) -> &W {
        &self.workflow
    }

    fn broadcaster_for(&self, unit: &UnitId) -> broadcast::Sender<Event<W>> {
        self.broadcasters
            .entry(unit.clone())
            .or_insert_with(|| broadcast::channel(BROADCAST_CAPACITY).0)
            .clone()
    }

    /// Override the lock-acquisition timeout (default 30 s).
    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// Override the lock lease (default 5 min).
    #[must_use]
    pub fn with_lock_lease(mut self, lease: Duration) -> Self {
        self.lock_lease = lease;
        self
    }

    /// Storage adapter backing this repository.
    #[must_use]
    pub fn storage(&self) -> &FileSystemStorage {
        &self.storage
    }

    /// Parse JSONL lines into (header, events) pairs. The header, if
    /// present, must appear on the first line.
    fn parse_lines(
        lines: &[String],
        report: &LoadReport,
    ) -> Result<(Option<Header>, Vec<Event<W>>), RepositoryError> {
        if let Some(first_span) = report.first_corruption() {
            return Err(RepositoryError::Corrupted { line: first_span.start });
        }
        let mut header = None;
        let mut events = Vec::with_capacity(lines.len());
        for (idx, raw) in lines.iter().enumerate() {
            let value: serde_json::Value = serde_json::from_str(raw)
                .map_err(|_| RepositoryError::Corrupted { line: (idx + 1) as u64 })?;
            let is_header = value.get("kind").and_then(|v| v.as_str()) == Some("__header__");
            if is_header {
                header = Some(
                    serde_json::from_value::<Header>(value)
                        .map_err(|_| RepositoryError::Corrupted { line: (idx + 1) as u64 })?,
                );
                continue;
            }
            let event: Event<W> = serde_json::from_value(value).map_err(RepositoryError::Codec)?;
            events.push(event);
        }
        Ok((header, events))
    }

    fn header_line(&self) -> Result<String, RepositoryError> {
        let salt = self.workflow.fingerprint_salt();
        let header = Header {
            schema_version: self.workflow.schema_version(),
            workflow: compact_str::CompactString::from(self.workflow.name().as_ref()),
            fingerprint_salt: compact_str::CompactString::from(base64_of(&salt)),
        };
        serde_json::to_string(&header).map_err(RepositoryError::Codec)
    }

    /// Refuse to append when the stored `fingerprint_salt` doesn't
    /// match the current workflow's salt. Silent drift would cause
    /// dedup to miss duplicates (or create spurious ones) once the
    /// salt changed. See `.claude/rules/fingerprint.md`.
    fn check_header_salt(&self, header: Option<&Header>) -> Result<(), RepositoryError> {
        let Some(h) = header else { return Ok(()) };
        let salt = self.workflow.fingerprint_salt();
        let current = base64_of(&salt);
        if h.fingerprint_salt.as_str() == current.as_str() {
            Ok(())
        } else {
            Err(RepositoryError::SaltMismatch { stored: h.fingerprint_salt.to_string(), current })
        }
    }

    /// One attempt at load → precondition → commit. Returns
    /// [`CommitOutcome::CasMismatch`] on optimistic-CAS failure so the
    /// outer loop can re-load and retry. All other failures propagate
    /// as `RepositoryError`.
    async fn commit_batch(
        &self,
        unit: &UnitId,
        proposals: &[Proposal<W>],
        mode: AppendMode,
    ) -> Result<CommitOutcome<W>, RepositoryError> {
        let (lines, report) = self.storage.load(unit).await.map_err(storage_err)?;
        let (existing_header, events) = Self::parse_lines(&lines, &report)?;
        self.check_header_salt(existing_header.as_ref())?;
        let existing_fingerprints: Vec<Fingerprint> = events
            .iter()
            .map(|e| fingerprint_event(&self.workflow, e).map_err(RepositoryError::Codec))
            .collect::<Result<_, _>>()?;

        let mut out_lines: Vec<String> = Vec::with_capacity(proposals.len() + 1);
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let mut used: Vec<Fingerprint> = existing_fingerprints;
        let mut working_events: Vec<Event<W>> = events.clone();
        let mut last_at: Option<Timestamp> = working_events.last().map(|e| e.at);

        for proposal in proposals {
            let proposal = proposal.clone();
            // 1. Dedup — idempotent replay is silent.
            let fp =
                fingerprint_proposal(&self.workflow, &proposal).map_err(RepositoryError::Codec)?;
            if used.contains(&fp) {
                rejected.push(RejectedProposal::new(proposal, "duplicate"));
                continue;
            }
            // 2. Preconditions — body + extension — evaluated against the
            // working log (i.e. including earlier accepts in this batch).
            let working_log = Log::from_events(unit.clone(), working_events.clone());
            let ctx =
                knotch_kernel::precondition::AppendContext::<W>::new(&self.workflow, &working_log);
            if let Err(err) = proposal.body.check_precondition(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal::new(proposal, err.to_string()));
                continue;
            }
            if let Err(err) = proposal.extension.check_extension::<W>(&ctx) {
                if matches!(mode, AppendMode::AllOrNothing) {
                    return Err(RepositoryError::Precondition(err));
                }
                rejected.push(RejectedProposal::new(proposal, err.to_string()));
                continue;
            }
            // 3. Monotonic timestamp — self-healing against clock drift.
            let at = stamp_monotonic(&SystemClock, last_at);
            let event = Event {
                id: EventId::new_v7(),
                at,
                causation: proposal.causation.clone(),
                extension: proposal.extension.clone(),
                body: proposal.body.clone(),
                supersedes: proposal.supersedes,
            };
            used.push(fp);
            last_at = Some(at);
            working_events.push(event.clone());
            let line = serde_json::to_string(&event).map_err(RepositoryError::Codec)?;
            out_lines.push(line);
            accepted.push(event);
        }

        if matches!(mode, AppendMode::AllOrNothing) && !rejected.is_empty() {
            return Ok(CommitOutcome::Rolled {
                report: AppendReport::new(Vec::new(), rejected),
            });
        }

        let header_missing = existing_header.is_none() && lines.is_empty();
        let expected_len = lines.len() as u64;
        if header_missing {
            out_lines.insert(0, self.header_line()?);
        }
        match self.storage.append(unit, expected_len, out_lines).await {
            Ok(()) => Ok(CommitOutcome::Committed { accepted, rejected }),
            Err(StorageError::LogMutated { expected, on_disk }) => {
                Ok(CommitOutcome::CasMismatch { expected, on_disk })
            }
            Err(e) => Err(storage_err(e)),
        }
    }

    /// Drive `commit_batch` with bounded exponential-backoff retry on
    /// optimistic-CAS mismatch. On `CAS_MAX_RETRIES` consecutive
    /// mismatches, surfaces the final `StorageError::LogMutated` to
    /// the caller.
    async fn commit_with_cas_retry(
        &self,
        unit: &UnitId,
        proposals: &[Proposal<W>],
        mode: AppendMode,
    ) -> Result<CommitOutcome<W>, RepositoryError> {
        for attempt in 0..CAS_MAX_RETRIES {
            let outcome = self.commit_batch(unit, proposals, mode).await?;
            match outcome {
                CommitOutcome::CasMismatch { expected, on_disk }
                    if attempt + 1 < CAS_MAX_RETRIES =>
                {
                    tokio::time::sleep(cas_backoff(attempt)).await;
                    tracing::debug!(
                        unit = unit.as_str(),
                        attempt = attempt + 1,
                        expected,
                        on_disk,
                        "knotch: CAS mismatch — retrying after backoff"
                    );
                }
                CommitOutcome::CasMismatch { expected, on_disk } => {
                    return Err(storage_err(StorageError::LogMutated { expected, on_disk }));
                }
                other => return Ok(other),
            }
        }
        unreachable!("retry loop must terminate via return")
    }
}

/// Exponential backoff schedule for CAS retries: 25ms, 50ms, 100ms,
/// 200ms, 400ms. Total wait before final attempt: 775ms. Saturates
/// at `u32::MAX` multiplier rather than panicking on shift overflow.
fn cas_backoff(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
    CAS_BASE_DELAY.saturating_mul(factor)
}

fn base64_of(bytes: &[u8]) -> String {
    use base64_of::encode;
    encode(bytes)
}

/// Tiny inline base64 encoder (stdlib doesn't ship one and pulling
/// `base64` for a single header field would inflate the dep graph).
mod base64_of {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub(super) fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        let mut chunks = input.chunks_exact(3);
        for chunk in &mut chunks {
            let n = u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8 | u32::from(chunk[2]);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push(TABLE[(n & 0x3f) as usize] as char);
        }
        let rem = chunks.remainder();
        match rem.len() {
            0 => {}
            1 => {
                let n = u32::from(rem[0]) << 16;
                out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
                out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
                out.push('=');
                out.push('=');
            }
            2 => {
                let n = u32::from(rem[0]) << 16 | u32::from(rem[1]) << 8;
                out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
                out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
                out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
                out.push('=');
            }
            _ => unreachable!(),
        }
        out
    }
}

fn storage_err(e: StorageError) -> RepositoryError {
    RepositoryError::Storage(Box::new(e))
}

fn lock_err(e: knotch_lock::LockError) -> RepositoryError {
    RepositoryError::Lock(Box::new(e))
}

impl<W: WorkflowKind> Repository<W> for FileRepository<W> {
    fn workflow(&self) -> &W {
        &self.workflow
    }

    async fn append(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
    ) -> Result<AppendReport<W>, RepositoryError> {
        let _guard =
            self.lock.acquire(unit, self.lock_timeout, self.lock_lease).await.map_err(lock_err)?;

        match self.commit_with_cas_retry(unit, &proposals, mode).await? {
            CommitOutcome::Committed { accepted, rejected } => {
                let tx = self.broadcaster_for(unit);
                for event in &accepted {
                    let _ = tx.send(event.clone());
                }
                Ok(AppendReport::new(accepted, rejected))
            }
            CommitOutcome::Rolled { report } => Ok(report),
            CommitOutcome::CasMismatch { .. } => {
                unreachable!("commit_with_cas_retry surfaces the mismatch as an error")
            }
        }
    }

    async fn load(&self, unit: &UnitId) -> Result<Arc<Log<W>>, RepositoryError> {
        let (lines, report) = self.storage.load(unit).await.map_err(storage_err)?;
        let (header, events) = Self::parse_lines(&lines, &report)?;
        self.check_header_salt(header.as_ref())?;
        Ok(Arc::new(Log::from_events(unit.clone(), events)))
    }

    async fn subscribe(
        &self,
        unit: &UnitId,
        mode: SubscribeMode,
    ) -> Result<PinStream<SubscribeEvent<W>>, RepositoryError> {
        // In-process subscription — same-process writers broadcast
        // through `broadcaster_for(unit)`. Cross-process subscription
        // (file-watch) remains Phase 10 hardening.
        let tx = self.broadcaster_for(unit);
        let rx = tx.subscribe();
        // Snapshot history for replay modes.
        let history: Vec<Event<W>> = match mode {
            SubscribeMode::LiveOnly => Vec::new(),
            SubscribeMode::FromBeginning => {
                let log = Repository::load(self, unit).await?;
                log.events().to_vec()
            }
            SubscribeMode::FromEventId(anchor) => {
                let log = Repository::load(self, unit).await?;
                let idx = log.events().iter().position(|e| e.id == anchor).map_or(0, |i| i + 1);
                log.events()[idx..].to_vec()
            }
            _ => Vec::new(),
        };
        let stream = try_stream! {
            for evt in history {
                yield SubscribeEvent::Event(Box::new(evt));
            }
            let mut rx = rx;
            loop {
                match rx.recv().await {
                    Ok(evt) => yield SubscribeEvent::Event(Box::new(evt)),
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        match rx.recv().await {
                            Ok(evt) => {
                                let id = evt.id;
                                yield SubscribeEvent::Lagged { skipped, next: id };
                                yield SubscribeEvent::Event(Box::new(evt));
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        }
                    }
                }
            }
        };
        Ok(Box::pin(stream.map(|r: Result<_, std::convert::Infallible>| match r {
            Ok(item) => item,
            Err(e) => match e {},
        })))
    }

    fn list_units(&self) -> PinStream<Result<UnitId, RepositoryError>> {
        let base = self.storage.list_units();
        Box::pin(base.map(|r| r.map_err(storage_err)))
    }

    async fn with_cache(
        &self,
        unit: &UnitId,
        proposals: Vec<Proposal<W>>,
        mode: AppendMode,
        mutate_cache: CacheMutator,
    ) -> Result<AppendReport<W>, RepositoryError> {
        let _guard =
            self.lock.acquire(unit, self.lock_timeout, self.lock_lease).await.map_err(lock_err)?;

        // Read + mutate cache once, outside the retry loop. The mutator
        // is `FnOnce` by contract and does not depend on event state —
        // it projects cache-local intent, not log-derived state. Cache
        // write happens after the log append commits.
        //
        // The resume-cache is non-authoritative (constitution §I), so
        // a cache read error is never a reason to block an append. Fall
        // back to an empty map and let the mutator run; the mutator is
        // a pure projection that must tolerate the missing-cache case
        // anyway (same as NotFound on first use). Observer dedup folds
        // any duplicate work on rebuild into no-ops.
        let cache_map = match self.storage.read_cache(unit).await {
            Ok(map) => map,
            Err(err) => {
                tracing::warn!(
                    unit = unit.as_str(),
                    error = %err,
                    "knotch: resume-cache read failed — starting from empty, \
                     cache will rebuild on next load (log is authoritative)",
                );
                serde_json::Map::new()
            }
        };
        let mut cache = ResumeCache::from(cache_map);
        mutate_cache(&mut cache);

        match self.commit_with_cas_retry(unit, &proposals, mode).await? {
            CommitOutcome::Committed { accepted, rejected } => {
                // The log is the sole source of truth (constitution §I);
                // the resume-cache is a checkpoint that can safely lag
                // or be missing. If the cache write fails after the log
                // append succeeded, we keep the log and emit a warning
                // rather than propagating an error that would suggest
                // the append failed. On the next load, a missing-or-
                // stale cache reads as empty and the observer re-
                // processes the window it had already advanced past —
                // safe because fingerprint dedup turns the repeat into
                // idempotent no-ops.
                if let Err(err) = self.storage.write_cache(unit, cache.as_map().clone()).await {
                    tracing::warn!(
                        unit = unit.as_str(),
                        error = %err,
                        "knotch: resume-cache write failed after successful log append — \
                         cache will rebuild on next load (log is authoritative)"
                    );
                }
                let tx = self.broadcaster_for(unit);
                for event in &accepted {
                    let _ = tx.send(event.clone());
                }
                Ok(AppendReport::new(accepted, rejected))
            }
            CommitOutcome::Rolled { report } => Ok(report),
            CommitOutcome::CasMismatch { .. } => {
                unreachable!("commit_with_cas_retry surfaces the mismatch as an error")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_backoff_doubles_each_attempt() {
        assert_eq!(cas_backoff(0), Duration::from_millis(25));
        assert_eq!(cas_backoff(1), Duration::from_millis(50));
        assert_eq!(cas_backoff(2), Duration::from_millis(100));
        assert_eq!(cas_backoff(3), Duration::from_millis(200));
        assert_eq!(cas_backoff(4), Duration::from_millis(400));
    }

    #[test]
    fn cas_backoff_saturates_at_u32_overflow() {
        // Attempt = 31 would overflow `1 << attempt` as u32; saturating
        // arithmetic keeps the delay bounded instead of panicking.
        let _ = cas_backoff(31);
        let _ = cas_backoff(50);
    }
}
