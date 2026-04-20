# knotch-example-minimal

The smallest non-trivial knotch workflow.

- **Phases** — `Start` → `Done`
- **Milestone** — free-form `TaskId(CompactString)`
- **Gate** — `Review` checkpoint
- **Extension** — none (`()`)

## Run

```bash
cargo run -p knotch-example-minimal
```

## What to copy

When building your own workflow:

1. Derive `PhaseKind` / `MilestoneKind` / `GateKind` on your enums/newtypes — the macros generate the trait impls plus the `#[non_exhaustive]` markers.
2. Implement `WorkflowKind` on a zero-sized marker type. Four associated types (`Phase`, `Milestone`, `Gate`, `Extension`), two constants (`NAME`, `SCHEMA_VERSION`), and `required_phases(scope)` are required.
3. Override `is_terminal_status` when your workflow has archival states — this drives the Phase × Status cross-invariant (see `.claude/rules/preconditions.md`).
4. Everything else (Repository, Proposal, Causation) comes from `knotch-kernel` / `knotch-storage` unchanged.
