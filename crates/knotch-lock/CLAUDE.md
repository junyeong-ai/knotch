# knotch-lock

Per-unit advisory locks. Zero `unsafe`. Two-layer:

1. **Intra-process** — a `DashMap<UnitId, Arc<tokio::sync::Mutex>>`
   serializes concurrent tasks inside one process. Necessary because
   fcntl advisory locks are per-process and would not otherwise
   prevent same-process races.
2. **Cross-process** — `fs4::tokio::AsyncFileExt` (rustix-backed)
   exclusive lock on `<unit>/.lock`.

## Module map

| Module | Owns |
|---|---|
| `lib` | `Lock` port — `acquire(unit, timeout, lease) -> LockGuard` |
| `file_lock` | `FileLock` + `LockGuard` |
| `metadata` | `LockMetadata { owner, acquired_at, lease }` sidecar (`<unit>/.lock.meta`) |
| `error` | `LockError` (Timeout / Contended / Io / MalformedMetadata) |

## Stale reclaim

A lock whose metadata satisfies either

- `now > acquired_at + lease`, OR
- `!rustix::process::test_kill_process(pid).is_ok()`

is treated as stale. The next acquirer takes it and the returned
`LockGuard::was_reclaimed()` is `true` — the Repository surfaces
that as `ReconcileFailed { anchor: RetryAnchor::Lock(pid),
kind: FailureKind::StaleLockReclaimed }`.

## Do not

- Call `unsafe` FFI — see @../../.claude/rules/no-unsafe.md. Every
  syscall has a safe `rustix` wrapper.
- Rely on the metadata sidecar for concurrency — it is observability
  data. The actual mutual exclusion comes from `fs4`.
