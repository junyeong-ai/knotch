---
name: knotch-query
description: Read the current state of a unit — current phase, status, shipped milestones, cost — or find units that match a predicate. Works for active, archived, abandoned, and handed-off units alike (projections are pure historical reads). Use when the agent needs a fact before planning its next action.
paths:
  - "crates/**"
  - ".knotch/**"
allowed-tools: Bash(knotch show *) Bash(knotch current) Bash(knotch unit list) Bash(knotch log *)
---

# knotch-query

Projections are pure. Calling them is free (O(n) on log length, no
I/O beyond the load). Prefer a projection over re-deriving state
from raw events.

## Built-in projections (`knotch_kernel::project`)

| Function | Returns | Use for |
|---|---|---|
| `current_phase(log)` | `Option<W::Phase>` | "What should I work on next?" |
| `current_status(log)` | `Option<StatusId>` | "Has this been archived / abandoned?" |
| `shipped_milestones(log)` | `Vec<W::Milestone>` | "Which user stories are done?" |
| `effective_events(log)` | `Vec<Event<W>>` | Supersede-aware view of the log |
| `total_cost(log)` | `Cost` | Aggregate LLM spend for this unit |

All are pure fns. They take `&Log<W>`; obtain the log with
`repo.load(&unit).await?`.

## Cross-unit queries (`knotch-query`)

```rust
use knotch_query::QueryBuilder;
use knotch_kernel::StatusId;

let units = QueryBuilder::<W>::new()
    .where_status(StatusId::new("in_review"))
    .since(jiff::Timestamp::from_second(1_700_000_000)?)
    .limit(20)
    .execute(&repo)
    .await?;
```

Filters AND-combine. `execute` walks `repo.list_units()` and loads
each. For large workspaces, prefer `where_status` or `where_phase`
early — they short-circuit per-unit.

## Workflow-specific projections

Cost / summary helpers are workflow-specific — the canonical
`Knotch` workflow ships only the core projections above. Adopter
workflows that want LLM-summary or USD-rollup projections add
them on their own `WorkflowKind` impl; see
`examples/workflow-vibe-case-study/src/lib.rs` for a reference
implementation (`total_usd`, `total_tokens`, `summary_for_llm`).

## Custom projections

Implement `Projection<W>` (trait in `knotch_kernel::project`):

```rust
pub struct UnshippedGates;
impl<W: WorkflowKind> Projection<W> for UnshippedGates {
    type View = Vec<W::Gate>;
    fn project(log: &Log<W>) -> Self::View { ... }
}
```

Projections must be pure: no I/O, no `Timestamp::now()`, no hidden
state. Verified by policy, not by compiler.

## Do not

- Walk `log.events()` by hand if a projection already exists.
- Cache the `Arc<Log<W>>` returned by `load` — projections are
  cheap; a stale log hides fresh events.
- Call `project_*` inside `repo.append`'s precondition — the
  precondition already sees the right snapshot via `AppendContext`.
