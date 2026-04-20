---
name: knotch-transition
description: Transition the active unit to a new lifecycle status. Use when declaring the unit shipped, archived, abandoned, handed-off, or deprecated. Terminal transitions require every required phase to be resolved, or a forced override with rationale.
argument-hint: "<target-status> [--forced --reason <text>]"
allowed-tools: Bash(knotch transition *) Bash(knotch current)
---

# knotch-transition

Move the unit to a new status.

## Common status values

Canonical `Knotch` workflow vocabulary:

- `draft`, `in_progress`, `in_review`, `shipped` — non-terminal,
  always allowed.
- Terminal: `archived`, `abandoned`, `superseded`, `deprecated`.

Adopter-forked workflows may expose a different status set;
`knotch transition` warns when the target is outside
`W::known_statuses()` but proceeds (the kernel's `StatusId` is
open-universe).

## Syntax

```bash
knotch transition <target>
knotch transition <target> --forced --reason <rationale>
```

Examples:

```bash
knotch transition in_review
knotch transition archived --forced --reason "upstream feature dropped"
```

## What fires

`EventBody::StatusTransitioned { target, forced, rationale }`.

## What to check first

- `knotch current` — confirm the active unit and its current status.
- Target status must differ from current. No-op transitions are
  rejected with `NoOpStatusTransition`.
- For non-forced terminal transitions: every phase in
  `W::required_phases(scope)` must be completed or skipped
  (Phase × Status cross-invariant — see
  `@.claude/rules/preconditions.md`).
- For forced transitions: `--reason` is mandatory. Without it, the
  precondition fails with `ForcedWithoutRationale`.

## When NOT to use

- Shipping a milestone — that's `git commit` plus the
  `verify-commit` hook.
- Marking a phase — use `/knotch-mark`.
- Undoing a prior event — use `knotch supersede <event-id>`.
