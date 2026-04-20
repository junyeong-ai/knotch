---
paths:
  - "crates/knotch-kernel/src/repository.rs"
  - "crates/knotch-storage/src/file_repository.rs"
  - "crates/knotch-testing/src/repo.rs"
---

# `Repository::append` flow

The exact contract every adapter must satisfy. Deviations are bugs.

## Entry

```rust
async fn append(
    &self,
    unit: &UnitId,
    proposals: Vec<Proposal<W>>,
    mode: AppendMode,
) -> Result<AppendReport<W>, RepositoryError>
```

## Steps (performed under the unit's lock)

1. **Acquire lock** — `FileLock` for cross-process, per-unit
   `tokio::sync::Mutex` for intra-process serialization.
2. **Load snapshot** — `storage.load(unit)` → parse header + events.
3. **Compute existing fingerprints** — via `fingerprint_event`.
4. For each proposal:
   a. **Dedup** — reject as `"duplicate"` if fingerprint already present.
   b. **Precondition** — `body.check_precondition(&AppendContext)`.
      `AllOrNothing` → surface as `RepositoryError::Precondition`.
      `BestEffort` → push to `rejected` with display of the error.
   c. **Extension precondition** — `extension.check_extension(ctx)`.
   d. **Monotonic `at`** — `Timestamp::now()` must be ≥ last event's
      `at`. Violation → `NonMonotonic` or `rejected`.
   e. **Stamp** — `EventId::new_v7()`, `at = Timestamp::now()`.
   f. **Append to working log** — so next proposal sees it in its
      precondition window.
5. **All-or-nothing rollback** — if any rejection exists under
   `AllOrNothing`, discard the working log and return
   `AppendReport { accepted: [], rejected }`.
6. **Commit to storage** — `storage.append(unit, expected_len, lines)`.
   Uses optimistic CAS — another writer extending the log between
   load and append yields `StorageError::LogMutated`.
7. **Fanout to subscribers** — per-unit broadcast `Sender::send`.
   No-receivers ignored.

## `with_cache` variant

Identical flow with one extra step before step 4:

- **Load cache → clone → mutate** — the mutator operates on the
  clone. On precondition failure the adapter leaves the original
  cache untouched. On success, the log append commits first, then
  the cache write is attempted. A cache-write failure after a
  successful log append is logged at `warn!` and swallowed — the
  log is the sole source of truth (constitution §I), the cache
  rebuilds on next load, and observer dedup turns the repeat
  proposals into no-ops.

See `crates/knotch-testing/src/repo.rs::with_cache` for the
canonical shape.

## Adapter invariants — all enforced by the adapter, not the kernel

The monotonic-`at` check, the optimistic CAS on `expected_len`,
the cache handling above, and the lock-held append are **adapter-
layer contracts**: the kernel precondition engine dispatches on
`EventBody` variants, which is independent of envelope metadata
(timestamp, cache state, fingerprints). Every new `Storage` /
`Repository` adapter re-implements the four items below and
covers each with a regression test:

1. Acquire the unit's lock before reading storage.
2. Reject proposals whose stamped `at` is earlier than the log's
   last-event `at` with `RepositoryError::NonMonotonic`.
3. Refuse cross-process writes that observed a stale log via
   optimistic CAS on the line count — `StorageError::LogMutated`
   signals retry.
4. Treat the resume-cache as non-authoritative — never error out
   when the cache write fails after the log append succeeded.

## What adapters MUST NOT do

- Call `storage.append` outside the unit lock.
- Skip precondition evaluation.
- Re-order proposals before committing (determinism).
- Persist partial batches under `AllOrNothing`.
- Send to the broadcast channel before `storage.append` returns `Ok`.

## What agents submitting proposals MUST do

- Accept that a `"duplicate"` rejection is the idempotent-replay
  success signal.
- Treat `RepositoryError::NonMonotonic` as a caller bug (check
  clock skew before retry).
- Use `AllOrNothing` for multi-proposal invariants; `BestEffort` for
  independent batches.
