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
//! itself keeps failing (network to VCS, bad config), the queue
//! grows every time a PostToolUse retry fails. `enqueue_raw` emits
//! a `tracing::warn!` when the queue size crosses
//! [`QUEUE_WARN_THRESHOLD`] so operators see the signal instead of
//! silent growth. The queue never drops entries on its own — doing
//! so would lose ledger events. `knotch reconcile --prune` and
//! `--prune-older <HOURS>` exist for explicit operator cleanup.

use std::path::Path;

use knotch_kernel::{AppendMode, Proposal, Repository, UnitId, WorkflowKind};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::HookError;

/// Queue-size threshold at which `enqueue_raw` emits a warning log.
/// Crossing this means `SessionStart` auto-drain is failing and the
/// operator needs to act. Deliberately generous so short transient
/// bursts don't spam logs; still low enough to signal long before
/// disk-space issues.
pub const QUEUE_WARN_THRESHOLD: usize = 100;

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
pub fn enqueue_raw(
    queue_dir: &Path,
    unit: &UnitId,
    proposal_json: serde_json::Value,
    reason: &str,
) -> Result<(), HookError> {
    std::fs::create_dir_all(queue_dir)?;
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
    // auto-drain isn't keeping up, but we must never lose an
    // unprocessed event on our own (that would violate the
    // append-only-log-is-truth invariant — the queue is how we
    // shuttle events that couldn't land synchronously).
    if let Ok(size) = queue_size(queue_dir) {
        if size >= QUEUE_WARN_THRESHOLD {
            tracing::warn!(
                queue_dir = %queue_dir.display(),
                size,
                threshold = QUEUE_WARN_THRESHOLD,
                "knotch queue backpressure: SessionStart auto-drain is not keeping up — \
                 run `knotch reconcile` to drain, or `knotch reconcile --prune-older <HOURS>` \
                 to TTL out entries that will never succeed",
            );
        }
    }
    Ok(())
}

/// Typed convenience wrapper when `Proposal<W>` is `Serialize`.
pub fn enqueue<W>(
    queue_dir: &Path,
    unit: &UnitId,
    proposal: &Proposal<W>,
    reason: &str,
) -> Result<(), HookError>
where
    W: WorkflowKind,
    Proposal<W>: Serialize,
{
    let json = serde_json::to_value(proposal)?;
    enqueue_raw(queue_dir, unit, json, reason)
}

/// Count how many entries are currently queued.
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
