---
paths:
  - "crates/knotch-kernel/src/event.rs"
  - "crates/knotch-kernel/src/precondition.rs"
  - "crates/knotch-storage/src/file_repository.rs"
  - "crates/knotch-testing/src/repo.rs"
---

# Append-time preconditions

`EventBody::check_precondition(&AppendContext<W>)` is the kernel's
policy pivot. Every Repository adapter calls it **inside** the unit
lock, against a **freshly-loaded** log snapshot.

## Variant → check

| Body | Rejects when |
|---|---|
| `UnitCreated` | log is non-empty |
| `PhaseCompleted` | same phase already completed / skipped; artifact missing (if `fs` probe present) |
| `PhaseSkipped` | phase refused the reason (`PhaseKind::is_skippable`); same phase already resolved |
| `MilestoneShipped` | non-implementation commit kind; `CommitStatus::Missing`; VCS says `Missing` (if `vcs` probe present); VCS says `Pending` while caller claims `Verified`; milestone already shipped |
| `MilestoneReverted` | milestone not in shipped set; revert commit unverifiable |
| `MilestoneVerified` | no matching prior `MilestoneShipped { status: Pending }` |
| `GateRecorded` | rationale shorter than `W::min_rationale_chars()`; any gate in `W::Gate::prerequisites` is absent from the log (`GateOutOfOrder`) |
| `StatusTransitioned` | target == current status (no-op); forced without rationale; non-forced terminal transition with required phases unresolved |
| *(any non-`EventSuperseded` body)* | unit is already in a terminal status — archived / abandoned / superseded units are immutable, so the only admissible append is `EventSuperseded` to roll back the transition |
| `ReconcileFailed` | attempt ≤ prior max for same anchor (non-monotonic) |
| `ReconcileRecovered` | no prior failure for anchor |
| `EventSuperseded` | target absent; target already superseded (combined with append-only construction this rules out supersede cycles — a new event cannot reach back to reference an unappended predecessor) |

Taxonomy: `crates/knotch-kernel/src/error.rs::PreconditionError`.

## Phase skipping — two orthogonal mechanisms

`knotch` distinguishes two ways a phase can be "not run":

- **Scope-based omission** — `W::required_phases(scope)` simply
  doesn't list the phase for that scope. Projections treat the
  absent phase as already resolved; no event is needed. This is
  the canonical mechanism (used by the `Knotch` workflow and
  every shipped case-study fork).
- **Explicit `PhaseSkipped` event** — an agent decides mid-flight
  that a required phase should be skipped with a specific reason.
  The precondition asks `PhaseKind::is_skippable(&reason)`; the
  default impl refuses every reason. Opt-in per phase.

Use scope-based omission when the decision is known up-front and
applies uniformly to every unit with that scope. Use explicit
`PhaseSkipped` when the decision is runtime-conditional and the
audit trail needs the reason recorded alongside the other events.

## Phase × Status cross-invariant

Non-forced terminal transitions require every `W::required_phases(scope)`
to be resolved (completed or skipped). Workflows declare their
terminal set by overriding `WorkflowKind::is_terminal_status`.

Canonical workflow:
- `knotch_workflow::Knotch`: `{archived, abandoned, superseded, deprecated}`.

Adopter-forked workflows declare their own terminal set via
`W::is_terminal_status`.

## Extension-level hook

`W::Extension` may contribute preconditions via
`ExtensionKind::check_extension`. Evaluated after the body check
succeeds, so extension logic can assume body invariants hold. Default
impl accepts everything (identity).

## Order of evaluation

1. Envelope — dedup fingerprint, monotonic `at`.
2. `EventBody::check_precondition` — closed dispatch.
3. `ExtensionKind::check_extension` — user-contributed.
4. External probes only evaluated when `AppendContext::vcs` / `::fs` set.

## Reject vs error

Under `AppendMode::AllOrNothing`, a precondition failure is a
`RepositoryError::Precondition` (caller rolls back). Under
`BestEffort`, it becomes an entry in `AppendReport::rejected` with
`reason` set to the display of the error.

## Why it must be in-crate (sealed dispatch)

`EventBody<W>` is `#[non_exhaustive]`. The `match` for
`check_precondition` lives in the kernel itself, so adding a new
variant forces every Repository adapter to re-compile. No silent
bypass.
