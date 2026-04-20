---
name: knotch-gate
description: Record a gate decision with long-form rationale on the active unit. The canonical Knotch workflow ships gates G0..G4 (Scope / Clarify / Plan / Review / Drift) with an 8-char `min_rationale_chars` floor — too-short text rejects as `RationaleTooShort`. Adopter-forked workflows may expose different gate vocabularies.
argument-hint: "<gate-id> <decision> <rationale>"
allowed-tools: Bash(knotch gate *) Bash(knotch current)
---

# knotch-gate

Record a gate event — your judgment about whether the unit meets
the checkpoint criteria.

## Decisions

- `approved` — passes the gate. Most common.
- `rejected` — fails the gate. Follow-up work required.
- `needs_revision` — partial; specific changes requested.
- `deferred` — postponed.

## Syntax

```bash
knotch gate <gate-id> <decision> <rationale>
```

Examples:

```bash
knotch gate g0-scope approved "scope fits a tiny unit — single file, no UX"
knotch gate g2-plan rejected "violates constitution §IV — plan imports tokio::fs"
knotch gate g3-review approved "LLM review pass clean; no blockers"
```

## What fires

`EventBody::GateRecorded { gate, decision, rationale }`.

## What to check first

- `knotch current` — confirm the active unit.
- Rationale must meet `W::min_rationale_chars()` — the canonical
  `Knotch` workflow uses the 8-char default.
- Gate id must match a variant of the active workflow's `W::Gate`
  enum. Canonical `Knotch` gates: `g0-scope`, `g1-clarify`,
  `g2-plan`, `g3-review`, `g4-drift`.
- Earlier gates in the ladder must already be recorded. Order:
  `G0 → G1 → G2 → G3 → G4`. The kernel enforces this at append
  time via `KnotchGate::prerequisites` — out-of-order proposals
  fail with `PreconditionError::GateOutOfOrder`.
