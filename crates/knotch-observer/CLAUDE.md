# knotch-observer

`Observer<W>` port + first-party observers. **Pure proposers** — an
observer never writes; it returns `Vec<Proposal<W>>` and the
reconciler composes them into one `Repository::append` call.

## Module map

| Module | Owns |
|---|---|
| `lib` | `Observer<W>` trait, `DynObserver<W>` dyn-compatible wrapper + blanket impl, `BoxObserveFuture` type alias |
| `context` | `ObserveContext<'a, W>` (unit / log snapshot / head / cache / cancel / budget / clock), `FsView`, `StdFsView`, `ObserveBudget` |
| `error` | `ObserverError` (Cancelled / BudgetExceeded / Vcs / Fs / Backend) |
| `git_log` | `GitLogObserver<V, W>` + `MilestoneResolver<W>` — walks `Vcs::log_since`, emits `MilestoneShipped` / `MilestoneReverted` |
| `artifact` | `ArtifactObserver<W>` + `PhaseScanner<W>` — emits `PhaseCompleted` when required artifacts land on disk |
| `pending_commit` | `PendingCommitObserver<V, W>` — promotes `MilestoneShipped { status: Pending }` to `MilestoneVerified` once VCS sees the commit |

## Observer contract

- **Pure over external state** — same VCS snapshot + same filesystem
  → same proposals. Fingerprint dedup handles replay; the observer
  does not filter.
- **Cancellable** — poll `ctx.cancel.is_cancelled()` between
  expensive steps; return `ObserverError::Cancelled` when tripped.
  Verified by `tests/cancel.rs`.
- **Budgeted** — respect `ctx.budget.max_proposals`; return
  `ObserverError::BudgetExceeded` rather than truncating silently.
- **Deterministic ordering** — the reconciler sorts the merged
  proposal batch by `(observer_name, kind_ordinal, kind_tag)`.
  Your observer's *internal* emission order does not affect the
  final append order, but keep it stable for test reproducibility.

## Extension recipe

**Write a new observer:**
1. Implement `Observer<W>`. Declare `name() -> &'static str` — this
   is the sort key.
2. Build proposals with `Causation::new(Source::Observer,
   Trigger::Observer { name: name.into() })`.
3. Do not store `UnitId` on the observer — read it from
   `ctx.unit`. Observers are workflow-wide.

## Do not

- Mutate external state (write to disk, push to git, call APIs).
  Observers are pure **proposers**; side effects belong to the
  adapter the observer wraps (e.g. `Vcs` for git reads).
- Re-implement fingerprint dedup or monotonic-`at` checks — the
  Repository handles both.
