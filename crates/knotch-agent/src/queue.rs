//! Reconciler queue — per-entry JSON files under `.knotch/queue/`.
//!
//! Each file is one failed append that the reconciler will drain on
//! the next run. The filename is the UUIDv7 of the entry, so
//! directory listing + lexicographic sort gives chronological order
//! with no external clock dependency.
//!
//! # Why individual files
//!
//! A single append-only log would require cross-process locking.
//! Claude Code fires hooks in parallel; writing one file per entry
//! sidesteps the race entirely — the worst case is two hooks writing
//! two differently-named files.
//!
//! # Backpressure
//!
//! A healthy queue drains on every `SessionStart`. If that hook
//! itself keeps failing (network to VCS, bad config), each
//! PostToolUse retry failure adds another entry. Two layers of
//! defense:
//!
//! 1. **Warning signal** — `enqueue_raw` emits a `tracing::warn!`
//!    when the queue size crosses [`QUEUE_WARN_THRESHOLD`], pointing
//!    the operator at `knotch reconcile` before disk pressure builds.
//! 2. **Hard cap** — `QueueConfig::max_entries` bounds the queue;
//!    the [`OverflowPolicy`] decides what happens when a new entry
//!    would exceed it. Operators choose between refusing the append
//!    (`Reject`, the default) and dropping the oldest entry to make
//!    room (`SpillOldest`).

use std::{path::Path, time::Duration};

use knotch_kernel::{
    AppendMode, Proposal, Repository, RepositoryError, UnitId, WorkflowKind,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::{error::HookError, orphan, output::HookOutput};

/// Queue-size threshold at which `enqueue_raw` emits a warning log.
/// Crossing this means `SessionStart` auto-drain is failing and the
/// operator needs to act. Deliberately generous so short transient
/// bursts don't spam logs; still low enough to signal long before
/// disk-space issues.
pub const QUEUE_WARN_THRESHOLD: usize = 100;

/// Default hard cap — 10k entries.  With the typical per-entry size
/// of ~1 KiB, this is ~10 MiB of queue data, large enough to survive
/// a day-long outage at a hook rate of a few per minute and small
/// enough to never threaten disk space on a developer machine.
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// What to do when `enqueue_raw` would push the queue past
/// [`QueueConfig::max_entries`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum OverflowPolicy {
    /// Refuse the new entry, surface `HookError::QueueFull`, leave
    /// the queue untouched. The conservative default — no event is
    /// ever dropped implicitly.
    #[default]
    Reject,
    /// Delete the lexicographically smallest UUIDv7 entry (i.e. the
    /// oldest), then write the new one. Chosen when the operator
    /// prefers freshness over total-retention.
    SpillOldest,
}

/// Operator-tunable queue policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueueConfig {
    /// Maximum number of queued entries. Defaults to
    /// [`DEFAULT_MAX_ENTRIES`].
    pub max_entries: usize,
    /// Behavior when `max_entries` would be exceeded.
    pub overflow: OverflowPolicy,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { max_entries: DEFAULT_MAX_ENTRIES, overflow: OverflowPolicy::default() }
    }
}

/// On-disk queue entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct QueueEntry {
    /// Target unit slug.
    pub unit: String,
    /// Fully-materialized proposal JSON — replayed as-is.
    pub proposal: serde_json::Value,
    /// ISO-8601 timestamp when this entry was queued.
    pub queued_at: String,
    /// Why the original append failed — informational only.
    pub reason: String,
}

/// Write a new queue entry. Filename uniqueness comes from UUIDv7;
/// no locking required.
///
/// The caller provides the proposal (already serialized to JSON) so
/// this function does not need `Proposal<W>: Serialize` bound.
///
/// # Errors
///
/// - [`HookError::QueueFull`] when [`QueueConfig::max_entries`] would
///   be exceeded under [`OverflowPolicy::Reject`].
/// - [`HookError::Io`] / [`HookError::Json`] on filesystem or
///   serialization failure.
pub fn enqueue_raw(
    queue_dir: &Path,
    unit: &UnitId,
    proposal_json: serde_json::Value,
    reason: &str,
    config: &QueueConfig,
) -> Result<(), HookError> {
    std::fs::create_dir_all(queue_dir)?;

    // Enforce the hard cap before writing anything new. We count
    // under the queue dir directly rather than trusting a cached
    // value — another hook running concurrently may have already
    // changed the count.
    let current = queue_size(queue_dir)?;
    if current >= config.max_entries {
        match config.overflow {
            OverflowPolicy::Reject => {
                return Err(HookError::QueueFull { size: current, max: config.max_entries });
            }
            OverflowPolicy::SpillOldest => {
                // Drop as many of the oldest entries as needed to
                // bring us back under the cap. Normally this removes
                // exactly one, but if the queue grew past max (e.g.
                // the operator lowered the cap at runtime), we catch
                // up in one pass.
                let surplus = current.saturating_sub(config.max_entries) + 1;
                spill_oldest(queue_dir, surplus)?;
            }
        }
    }

    let uid = Uuid::now_v7();
    let path = queue_dir.join(format!("{uid}.json"));
    let entry = QueueEntry {
        unit: unit.as_str().to_owned(),
        proposal: proposal_json,
        queued_at: jiff::Timestamp::now().to_string(),
        reason: reason.to_owned(),
    };
    let body = serde_json::to_vec_pretty(&entry)?;
    crate::atomic::write(&path, &body)?;

    // Warn-don't-drop: operators need a signal when SessionStart
    // auto-drain isn't keeping up. The hard cap catches real
    // overflow; this warning catches the approach.
    if let Ok(size) = queue_size(queue_dir) {
        if size >= QUEUE_WARN_THRESHOLD {
            tracing::warn!(
                queue_dir = %queue_dir.display(),
                size,
                threshold = QUEUE_WARN_THRESHOLD,
                max = config.max_entries,
                "knotch queue backpressure: SessionStart auto-drain is not keeping up — \
                 run `knotch reconcile` to drain, or `knotch reconcile --prune-older <HOURS>` \
                 to TTL out entries that will never succeed",
            );
        }
    }
    Ok(())
}

/// Typed convenience wrapper when `Proposal<W>` is `Serialize`.
///
/// # Errors
///
/// See [`enqueue_raw`].
pub fn enqueue<W>(
    queue_dir: &Path,
    unit: &UnitId,
    proposal: &Proposal<W>,
    reason: &str,
    config: &QueueConfig,
) -> Result<(), HookError>
where
    W: WorkflowKind,
    Proposal<W>: Serialize,
{
    let json = serde_json::to_value(proposal)?;
    enqueue_raw(queue_dir, unit, json, reason, config)
}

/// Count how many entries are currently queued.
///
/// # Errors
///
/// Returns `HookError::Io` on directory-read failure.
pub fn queue_size(queue_dir: &Path) -> Result<usize, HookError> {
    if !queue_dir.exists() {
        return Ok(0);
    }
    let mut n = 0;
    for entry in std::fs::read_dir(queue_dir)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|e| e == "json") {
            n += 1;
        }
    }
    Ok(n)
}

/// Remove the `count` lexicographically smallest entries. Used by
/// [`OverflowPolicy::SpillOldest`]. Missing files are ignored (a
/// concurrent drain may have already removed them).
fn spill_oldest(queue_dir: &Path, count: usize) -> Result<(), HookError> {
    if count == 0 || !queue_dir.exists() {
        return Ok(());
    }
    let mut paths: Vec<_> = std::fs::read_dir(queue_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for path in paths.into_iter().take(count) {
        tracing::warn!(
            path = %path.display(),
            "knotch queue spill-oldest: dropping entry to make room for a newer one",
        );
        // `remove_file` races are benign: if another drainer already
        // removed it, we're fine.
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

/// Drain every queued proposal through `repo`. Each entry that
/// appends successfully (including the idempotent duplicate case) is
/// removed from disk; entries that still fail remain for the next
/// drain.
///
/// Entries are processed in lexicographic filename order, which is
/// chronological because filenames are UUIDv7.
///
/// # Errors
/// I/O errors during directory listing bubble up. Individual entry
/// failures are logged and skipped (the queue is advisory; one bad
/// entry must not stall the rest).
pub async fn drain<W, R>(queue_dir: &Path, repo: &R) -> Result<usize, HookError>
where
    W: WorkflowKind,
    R: Repository<W>,
    Proposal<W>: DeserializeOwned,
{
    if !queue_dir.exists() {
        return Ok(0);
    }
    let mut paths: Vec<_> = std::fs::read_dir(queue_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();

    let mut drained = 0;
    for path in paths {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    "queue drain: read failed: {err}"
                );
                continue;
            }
        };
        let entry: QueueEntry = match serde_json::from_str(&raw) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    "queue drain: entry parse failed: {err} — leaving on disk for inspection"
                );
                continue;
            }
        };
        let unit = UnitId::new(entry.unit.clone());
        let proposal: Proposal<W> = match serde_json::from_value(entry.proposal) {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    "queue drain: proposal deserialize failed: {err} — wrong preset?"
                );
                continue;
            }
        };
        match repo.append(&unit, vec![proposal], AppendMode::BestEffort).await {
            Ok(_) => {
                let _ = std::fs::remove_file(&path);
                drained += 1;
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    "queue drain: append still failing: {err}"
                );
            }
        }
    }
    Ok(drained)
}

/// Number of append attempts for `post_tool_append`. The first is the
/// initial try; the remaining attempts back off exponentially.
pub const POST_TOOL_MAX_ATTEMPTS: u32 = 3;

/// Base delay between `post_tool_append` retries — 50 ms, then 200 ms,
/// then 800 ms (4× factor). Total wait before giving up: 1 s.
pub const POST_TOOL_BASE_DELAY: Duration = Duration::from_millis(50);

/// Backoff factor between attempts. Matches the hook-integration.md
/// schedule (50 / 200 / 800 ms).
const POST_TOOL_BACKOFF_FACTOR: u32 = 4;

fn post_tool_backoff(attempt: u32) -> Duration {
    // attempt = 0 → base, 1 → base×4, 2 → base×16, etc.
    let factor = POST_TOOL_BACKOFF_FACTOR.checked_pow(attempt).unwrap_or(u32::MAX);
    POST_TOOL_BASE_DELAY.saturating_mul(factor)
}

/// Operator-facing context carried alongside a `post_tool_append`
/// call. Keeps the helper signature narrow and lets the caller pin
/// every side-channel (queue dir, queue policy, orphan log path, hook
/// name tag, cwd for diagnostics) without a long parameter list.
#[derive(Debug, Clone, Copy)]
pub struct PostToolContext<'a> {
    /// Queue directory (usually `<project>/.knotch/queue`).
    pub queue_dir: &'a Path,
    /// Operator-tunable queue policy.
    pub queue_config: &'a QueueConfig,
    /// Home directory for the orphan log fallback (usually `$HOME`).
    pub home: &'a Path,
    /// Working directory at the time the hook fired — logged in the
    /// orphan record so operators can locate the affected project.
    pub cwd: &'a Path,
    /// Hook name tag, e.g. `"verify-commit"` / `"record-revert"`.
    /// Surfaces in the orphan log record and tracing spans.
    pub hook_name: &'a str,
}

/// Append a proposal under the PostToolUse contract
/// (`.claude/rules/hook-integration.md`):
///
/// 1. Retry up to [`POST_TOOL_MAX_ATTEMPTS`] times on transient
///    failures (`RepositoryError::Storage` / `Lock` / `Codec` /
///    `Corrupted`, `HookError::Io`). `RepositoryError::Precondition`
///    is **not** retried — preconditions are permanent policy
///    rejections (e.g. "milestone already shipped"), not transient.
/// 2. On retry exhaustion, enqueue via [`enqueue_raw`] so the
///    reconciler can drain on the next `SessionStart` /
///    `knotch reconcile`.
/// 3. On [`HookError::QueueFull`], fall back to the orphan log at
///    `~/.knotch/orphan.log` so the event is never silently dropped.
///
/// The helper returns [`HookOutput::Continue`] in every terminal path
/// (success, queued, orphaned) so the PostToolUse exit code stays 0
/// per the exit-code contract. Precondition rejections surface as
/// [`HookError::Repository`] for the CLI to decide on visibility.
pub async fn post_tool_append<W, R>(
    repo: &R,
    unit: &UnitId,
    proposal: Proposal<W>,
    ctx: PostToolContext<'_>,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    R: Repository<W>,
    Proposal<W>: Serialize,
{
    let mut last_err: Option<RepositoryError> = None;
    for attempt in 0..POST_TOOL_MAX_ATTEMPTS {
        match repo.append(unit, vec![proposal.clone()], AppendMode::BestEffort).await {
            Ok(_) => return Ok(HookOutput::Continue),
            Err(RepositoryError::Precondition(e)) => {
                // Permanent rejection — retry cannot help and the
                // queue would rediscover the same failure on drain.
                tracing::warn!(
                    hook = ctx.hook_name,
                    unit = unit.as_str(),
                    "{}: precondition rejected: {e}",
                    ctx.hook_name
                );
                return Err(HookError::Repository(RepositoryError::Precondition(e)));
            }
            Err(other) => {
                tracing::debug!(
                    hook = ctx.hook_name,
                    unit = unit.as_str(),
                    attempt = attempt + 1,
                    error = %other,
                    "{}: transient append failure, retrying",
                    ctx.hook_name,
                );
                last_err = Some(other);
                if attempt + 1 < POST_TOOL_MAX_ATTEMPTS {
                    tokio::time::sleep(post_tool_backoff(attempt)).await;
                }
            }
        }
    }

    // Retry exhausted — fall through to the queue. The caller's
    // failure signal is preserved in `reason` so operators can
    // correlate queue entries with the original error.
    let reason = last_err
        .as_ref()
        .map(|e| format!("{e}"))
        .unwrap_or_else(|| "retry exhausted without specific error".to_owned());

    match enqueue(ctx.queue_dir, unit, &proposal, &reason, ctx.queue_config) {
        Ok(()) => {
            tracing::info!(
                hook = ctx.hook_name,
                unit = unit.as_str(),
                "{}: append failed after retries, queued for reconcile",
                ctx.hook_name,
            );
            Ok(HookOutput::Continue)
        }
        Err(HookError::QueueFull { size, max }) => {
            // Queue cap hit under `OverflowPolicy::Reject` — the event
            // would otherwise be lost. Orphan-log it so the operator
            // sees the drop and can recover by hand.
            let orphan_reason = format!("queue-full size={size} max={max}; append reason: {reason}");
            orphan::log_orphan(
                ctx.home,
                &format!("knotch hook {}", ctx.hook_name),
                ctx.cwd,
                &orphan_reason,
            );
            tracing::warn!(
                hook = ctx.hook_name,
                unit = unit.as_str(),
                size,
                max,
                "{}: queue full — recorded in ~/.knotch/orphan.log",
                ctx.hook_name,
            );
            Ok(HookOutput::Continue)
        }
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unit() -> UnitId {
        UnitId::new("test-unit")
    }

    fn fill(dir: &Path, count: usize, config: &QueueConfig) {
        for i in 0..count {
            enqueue_raw(dir, &unit(), json!({ "seq": i }), "probe", config)
                .expect("enqueue under cap");
        }
    }

    #[test]
    fn reject_refuses_once_cap_reached() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("queue");
        let cfg = QueueConfig { max_entries: 3, overflow: OverflowPolicy::Reject };

        fill(&dir, 3, &cfg);
        assert_eq!(queue_size(&dir).unwrap(), 3);

        let err = enqueue_raw(&dir, &unit(), json!({ "seq": 3 }), "probe", &cfg).unwrap_err();
        assert!(matches!(err, HookError::QueueFull { size: 3, max: 3 }));
        assert_eq!(queue_size(&dir).unwrap(), 3, "queue unchanged on reject");
    }

    #[test]
    fn spill_oldest_drops_first_entry_when_full() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("queue");
        let cfg = QueueConfig { max_entries: 3, overflow: OverflowPolicy::SpillOldest };

        fill(&dir, 3, &cfg);
        // Small sleep so the next UUIDv7 timestamp is strictly later
        // than the oldest — same as agents writing real events.
        std::thread::sleep(std::time::Duration::from_millis(5));

        enqueue_raw(&dir, &unit(), json!({ "seq": 3 }), "probe", &cfg)
            .expect("spill overflow succeeds");

        assert_eq!(queue_size(&dir).unwrap(), 3);

        // The surviving entries should be [seq=1, seq=2, seq=3] — the
        // oldest (seq=0) was spilled.
        let mut remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        remaining.sort();
        let mut seqs = Vec::new();
        for path in &remaining {
            let raw = std::fs::read_to_string(path).unwrap();
            let entry: QueueEntry = serde_json::from_str(&raw).unwrap();
            seqs.push(entry.proposal["seq"].as_u64().unwrap());
        }
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[test]
    fn default_config_is_reject_with_ten_thousand_cap() {
        let cfg = QueueConfig::default();
        assert_eq!(cfg.max_entries, DEFAULT_MAX_ENTRIES);
        assert_eq!(cfg.overflow, OverflowPolicy::Reject);
    }

    #[test]
    fn spill_oldest_catches_up_when_queue_exceeds_cap() {
        // Simulates the operator lowering max_entries at runtime
        // while the queue already held more entries than the new cap.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("queue");
        let permissive = QueueConfig { max_entries: 100, overflow: OverflowPolicy::Reject };
        fill(&dir, 5, &permissive);
        assert_eq!(queue_size(&dir).unwrap(), 5);

        let tightened = QueueConfig { max_entries: 3, overflow: OverflowPolicy::SpillOldest };
        std::thread::sleep(std::time::Duration::from_millis(5));
        enqueue_raw(&dir, &unit(), json!({ "seq": 99 }), "probe", &tightened)
            .expect("spill catches up");

        // 5 existing + 1 new would be 6; surplus = 6 - 3 = 3, so
        // three oldest are removed, leaving seq=[3, 4, 99] (3 total).
        assert_eq!(queue_size(&dir).unwrap(), 3);
    }
}
