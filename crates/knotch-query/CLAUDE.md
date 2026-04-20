# knotch-query

CQRS read-side query builder — cross-unit search with an
AND-composed predicate set.

@../../.claude/rules/no-unsafe.md

## Surface

| Type / Fn | Role |
|---|---|
| `QueryBuilder<W>` | Declarative predicate builder. `.where_phase(p)` / `.where_status(s)` / `.where_milestone_shipped(m)` / `.since(ts)` / `.limit(n)` / `.execute(repo)`. |
| `QueryError` | Thin wrapper over `RepositoryError` — propagates backend failures verbatim. |

## Execution model

`execute` walks `repo.list_units()`, loads each log, and applies
the AND-combined filter set in memory. No storage-native indexing;
scale is O(num_units × events_per_unit). For workspaces with
thousands of units or hot dashboards, build a snapshot projection
instead of calling `QueryBuilder` on every request.

Short-circuit candidates — `where_status` and `where_phase` — are
evaluated first so the loader skips past units that can be
eliminated by the `Log` header alone.

## Extension recipe — add a new predicate

1. Add a `where_<field>` builder method on `QueryBuilder<W>`.
2. Store the predicate alongside the others; keep the builder
   `Clone` so branched queries stay cheap.
3. Apply the filter in `execute` AFTER the cheap `current_status`
   / `current_phase` short-circuits so expensive predicates don't
   run against disqualified units.
4. Cover with a pair of tests in `tests/query.rs` (positive +
   negative case).

## Do not

- Mutate the log — `QueryBuilder` is read-only. Changes go
  through `Repository::append` via the appropriate skill or hook.
- Expose the inner `Vec<UnitId>` directly — callers get an
  owned result; borrowing would tie the return to the builder's
  lifetime for no gain.
- Introduce a parallel API that duplicates the predicates — add
  a new filter method instead.
