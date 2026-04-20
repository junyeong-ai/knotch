# knotch-vcs

Git plumbing for the observer layer. Pure-Rust via `gix`.

## Module map

| Module | Owns |
|---|---|
| `lib` (`Vcs` trait) | `verify_commit(sha) -> CommitStatus`, `log_since(since, filter) -> Vec<Commit>`, `current_head`, `log_watermark`, default `detect_revert` |
| `commit` | `Commit`, `ParsedCommit`, `RevertLink`, `Watermark`. Re-exports `CommitStatus` from kernel. |
| `parse` | Conventional-Commits parser (winnow-based). Recognises `feat / fix / refactor / perf / docs / chore / test / ci / build / style / revert` + synthetic `Revert "..."` headers + `BREAKING CHANGE:` footers + `This reverts commit <sha>.` body lines. |
| `gix_vcs` | `GixVcs` — `ThreadSafeRepository` wrapped in `Arc`, each call takes a thread-local handle via `spawn_blocking`. |
| `error` | `VcsError` |

## Pending → Verified flow

`verify_commit` is tri-state: `Verified | Pending | Missing`. The
reconciler uses these to:

- emit `MilestoneShipped { status: Pending }` for commits visible
  in a referenced context but not yet fetched;
- later promote them via `PendingCommitObserver` (in
  `knotch-observer`) to `MilestoneVerified`.

`CommitStatus::Missing` is **never** stored in the event log — the
precondition (see @../../.claude/rules/preconditions.md) rejects
ship attempts for missing commits.

## Do not

- Depend on `git2` — `gix` is the single blessed backend.
- Parse commit bodies in presets — use `knotch_vcs::parse` so the
  grammar stays in one place.
