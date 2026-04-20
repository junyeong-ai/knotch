---
name: knotch-mark
description: Record a phase completion or skip on the active unit. The canonical `Knotch` workflow phases are `specify → plan → build → review → ship`; adopter-forked workflows may expose different names — run `knotch show --format json` on the active unit to see the current vocabulary. Does NOT handle milestones (those attach to commits via a `Knotch-Milestone:` git trailer — see "When NOT to use").
argument-hint: "completed <phase> | skipped <phase> --reason <text>"
allowed-tools: Bash(knotch mark *) Bash(knotch current) Bash(knotch show *) Bash(knotch log *)
---

# knotch-mark

Append a `PhaseCompleted` or `PhaseSkipped` event to the active
unit. The CLI does the actual `Repository::append`; this skill is
the thin instruction layer that picks the right form.

## Use when

- You finished a phase and want to record it (with optional artifact references).
- You've decided to skip a phase (most often on a narrow-scope unit).

## Discover the valid phase names first

Phase names are snake_case via serde. The canonical `Knotch`
workflow ships `specify / plan / build / review / ship`; if the
active unit is bound to a different workflow (adopter fork), run
the projection to see the canonical names:

```bash
knotch current                            # confirm active unit
knotch show <unit> --format json          # `current_phase` shows the *next* phase name
```

## Syntax

```bash
knotch mark completed <phase> [--artifact <path>]...
knotch mark skipped   <phase> --reason <text>
```

`<phase>` is the snake_case identifier from the active workflow's
`Phase` enum. If knotch rejects it with `unknown phase`, run
`knotch show --format json` to see the current value.

## What fires

- `EventBody::PhaseCompleted { phase, artifacts }` for `completed`.
- `EventBody::PhaseSkipped { phase, reason }` for `skipped`.

## What to check first

- `knotch current` — confirm the active unit.
- The phase must not already be completed or skipped (dedup will
  otherwise reject the append with `"duplicate"`, which is the
  idempotency success signal — not an error).
- For `skipped`: the reason must be one that
  `PhaseKind::is_skippable` accepts for that phase; otherwise the
  precondition rejects with `SkipRejected`.

## When NOT to use

- **Recording a milestone ship** — milestones are event-sourced off
  **git commits**, not this skill. Include a
  `Knotch-Milestone: <id>` trailer in your commit message (either
  as the last paragraph of `-m` or a second `-m` arg); the
  `verify-commit` hook then emits `MilestoneShipped` automatically.
  Commits without the trailer pass silently — knotch only records
  what the author explicitly named.
- Recording a gate rationale — use `/knotch-gate`.
- Moving to a terminal status — use `/knotch-transition`.
