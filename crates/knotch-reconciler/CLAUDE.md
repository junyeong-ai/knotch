# knotch-reconciler

Observer composition + deterministic merge + single `append` batch.

@../../.claude/rules/append-flow.md

## Module map

| Module | Owns |
|---|---|
| `lib` | `Reconciler<W, R>` + `ReconcilerBuilder` — parallel observer execution (`tokio::task::JoinSet`), deterministic sort, single batch append |
| `lib::kind_tag` (private) | Sort key `"{ordinal:02}.{tag}"` — delegates to `EventBody::kind_ordinal` / `kind_tag` (kernel-owned source of truth) |
| `error` | `ReconcileError` (Repository / JoinError) |
| `ReconcileReport<W>` | `append: AppendReport<W>` + `observer_errors: Vec<ObserverFailure>` |

## Contract

1. Acquire `Arc<Log<W>>` snapshot via `repo.load(&unit).await`.
2. Spawn every observer on a `JoinSet` with its own
   `tokio::time::timeout(observer.timeout(), observer.observe_boxed(ctx))`.
3. Collect proposals into `Vec<(observer_name, Proposal<W>)>`.
4. Sort: `by (observer_name, kind_ordinal, kind_tag)` — composed as `"{ordinal:02}.{tag}"`. Per constitution §IX; no body-debug tertiary because a single observer cannot legitimately emit two proposals with the same `(kind_ordinal, kind_tag)` that diverge in body (fingerprint dedup collapses equal-body proposals at the Repository boundary).
5. `repo.append(unit, proposals, self.append_mode)` — one call.

Determinism is a **runtime property** — two reconciles over the
same external state produce byte-identical logs (verified by
`tests/idempotent.rs`).

## When an observer times out

`tokio::time::timeout` returns `Err(Elapsed)` → the reconciler
records `ObserverFailure { observer, source: ObserverError::Cancelled }`
and moves on. Other observers continue. No partial state leaks into
the append because the sort happens after every join.

## Do not

- Call `repo.append` inside an observer — observers produce
  `Proposal<W>`s, not events.
- Re-sort proposals inside an observer — the reconciler sorts the
  merged batch. Observer-internal ordering is lost.
- Spawn tokio tasks that outlive the reconcile call — per-observer
  cancellation must propagate.
