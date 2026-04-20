# workflow-spec-driven-case-study

Reference fork of the canonical `knotch_workflow::Knotch` showing
a spec-driven lifecycle — `Specify → Design → Implement →
Review → Wrapup` — with a G0..G6 checkpoint-gate ladder (G4
deliberately skipped). Copy this source into your adopter repo
as a starting point when your shape matches.

@../../.claude/rules/constitution.md
@../../.claude/rules/preconditions.md

## Surface

| Type | Role |
|---|---|
| `SpecDriven` | The `WorkflowKind` marker |
| `SpecPhase` | `Specify / Design / Implement / Review / Wrapup` (Review is skipped in `Scope::Tiny`) |
| `StoryId` | Milestone — a single user-visible unit of work |
| `SpecGate` | `G0Scope / G1Clarify / G2Constitution / G3Analyze / G5Review / G6Drift` (no G4 — reserved) |
| `events::*` | Low-level `Proposal<W>` constructors (`unit_created`, `phase_completed`, `milestone_shipped`, `gate_recorded`, `status_transitioned`) — gate ordering is enforced by the kernel at append time, not by a preflight helper |
| `SpecGate::prerequisites` | Per-variant G0..G6 prerequisite graph; consumed by `EventBody::check_precondition` |

Terminal statuses: `archived`, `abandoned`, `superseded`,
`deprecated`. Canonical status vocabulary returned by
`known_statuses()`.

## Extension recipe

**Add a new gate:**

1. Extend `SpecGate` with the new variant. Pick an id that slots
   into the existing numeric ladder.
2. Add the variant to `SpecGate::prerequisites` and list the
   prior gates that must precede it.
3. Update `SpecGate::id` to emit the kebab-case identifier.
4. Add an ordering test against `tests/e2e.rs` that covers the
   happy path and the out-of-order rejection.
5. Teach `/knotch-gate` about the new id (update
   `.claude/skills/knotch-gate/SKILL.md` if preset-specific
   guidance is needed).

**Add a new phase:**

1. Extend `SpecPhase` in canonical order.
2. Update `PHASES_TINY` and `PHASES_STANDARD` constants.
3. Audit `required_phases` to confirm scope-specific inclusion.
4. Refresh `tests/phases.rs`.

## Do not

- Rename a phase / gate without bumping `SCHEMA_VERSION` — stored
  logs serialize these identifiers.
- Record a `MilestoneShipped` from inside this crate; milestones
  attach to commits through `knotch-agent::commit::verify`.
- Pick a terminal status outside `is_terminal_status` — the
  precondition engine rejects transitions that target unknown
  terminal vocabulary.
