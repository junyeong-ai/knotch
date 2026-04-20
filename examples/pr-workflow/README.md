# knotch-example-pr-workflow

Models an open-source pull-request lifecycle.

- **Phases** — `Draft` → `Review` → `Merged`
- **Milestones** — `PrId` (PR number or slug)
- **Gates** — `CodeReview`, `MaintainerApproval`
- **Terminal statuses** — `merged`, `closed`, `abandoned`
- **Rationale floor** — 16 chars (stricter than default 8)

## Run

```bash
cargo run -p knotch-example-pr-workflow
```

## Why this pattern works

- Each gate carries its rationale inline, so the event log itself is a PR audit trail — no sidecar comments.
- Splitting `CodeReview` and `MaintainerApproval` lets reviewer and maintainer sign off independently; precondition order comes from your own `WorkflowKind::parse_gate` policy or a preset-level helper.
- Raising `min_rationale_chars` to 16 prevents `/lgtm` one-offs from landing as gate decisions.
