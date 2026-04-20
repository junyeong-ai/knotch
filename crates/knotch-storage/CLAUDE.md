# knotch-storage

Byte-level persistence (`Storage` port) + `FileRepository<W>` — the
concrete `Repository` the CLI and presets use by default.

@../../.claude/rules/append-flow.md
@../../.claude/rules/fingerprint.md

## Module map

| Module | Owns |
|---|---|
| `lib` (`Storage` trait) | `load`, `append(expected_len, lines)` (optimistic CAS), `list_units`, `read_cache`, `write_cache` |
| `fs_storage` | `FileSystemStorage` — JSONL, atomic write-new + rename, line-count CAS |
| `atomic` | `atomic::write(path, contents)` — fsync file, rename, fsync parent dir (POSIX) / `MoveFileEx` + retry (Windows) |
| `file_repository` | `FileRepository<W>` — combines `FileSystemStorage` + `FileLock` + per-unit `broadcast::Sender` |
| `load_report` | `LoadReport`, `CorruptionSpan` |
| `error` | `StorageError` (Io / LogMutated / PermissionDenied / Backend) |

## `FileRepository<W>` invariants

- **In-process broadcast** — clones of the same `FileRepository`
  share the broadcast map via `Arc<DashMap<UnitId, Sender>>`. Two
  independently-constructed `FileRepository::new(same_root)` do
  **not** share subscribers. Prefer cloning a single instance.
- **Append fanout** — accepted events send to subscribers only
  **after** `storage.append` returns `Ok`. No premature notification.
- **Header on first append** — `Header::schema_version` / `workflow`
  / `fingerprint_salt` (base64) is written exactly once.
- **CAS via expected_len** — the Storage adapter verifies the
  on-disk line count matches what the Repository loaded; mismatch
  surfaces as `StorageError::LogMutated` and the batch retries (or
  fails under `AllOrNothing`).
- **Cache is non-authoritative** — the resume-cache is a checkpoint
  derived from the log, not a second source of truth (constitution
  §I). In `with_cache`, the cache write happens *after* the log
  append succeeds; if the cache write fails we emit a
  `tracing::warn!` and return `Ok` because the log is the authority
  and any observer that had advanced its cursor will simply rescan
  the same window next time — fingerprint dedup turns the repeated
  proposals into idempotent no-ops.

## Extension recipe

**Add a new `Storage` backend (redb, sqlite, s3):**
1. Implement the trait. Enforce `load` returning a `LoadReport`
   with corruption spans (not panicking).
2. Implement optimistic CAS — if the backend does not natively
   support it, wrap in a transaction.
3. Add an end-to-end test under `crates/knotch-storage/tests/`
   mirroring `file_repository.rs`.

## Do not

- Bypass `atomic::write` for log appends — non-atomic writes break
  `#[I] Event log is the only truth` under crash.
- Share `broadcast::Sender` across Repository *types* — per-`W`
  channel typing is intentional.
