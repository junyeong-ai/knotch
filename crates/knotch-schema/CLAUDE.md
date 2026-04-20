# knotch-schema

Two preset-agnostic policy libraries in one crate — presets pick
what they need; the kernel stays pure.

@../../.claude/rules/no-unsafe.md

## Module map

| Module | Owns |
|---|---|
| `frontmatter` | `FrontmatterSchema`, `FieldSchema`, `FieldType`, `SchemaError` — declarative validator for per-unit `spec.md` frontmatter. Presets declare required / enumerated / regex-matched fields; the schema validates any TOML/JSON object. |
| `lifecycle` | `LifecycleFsm`, `TransitionRequest`, `LifecycleError` — status FSM encoding terminal statuses and the Phase × Status cross-invariant (see `@../../.claude/rules/preconditions.md`) |

## When to pull in each

- Preset exposes a user-editable spec file → `FrontmatterSchema`
  to fail fast on missing / malformed fields.
- Preset wants machine-checked status transitions (vs. ad-hoc
  matching in `WorkflowKind::is_terminal_status`) →
  `LifecycleFsm` with `TransitionRequest`.

Both are opt-in. A preset may use neither, one, or both.

## Extension recipe

**Add a new `FieldType`:**

1. Extend `frontmatter::FieldType` — it is `#[non_exhaustive]`.
2. Match the new variant in `FrontmatterSchema::validate`.
3. Add a positive + negative test to `tests/frontmatter.rs`.

**Add a new lifecycle invariant:**

1. Extend `LifecycleFsm` with a new check.
2. Surface new failures via a new `LifecycleError` variant
   (`#[non_exhaustive]`).
3. Cite the ADR the invariant comes from in the variant doc.

## Do not

- Couple to a specific preset's `WorkflowKind` — both modules take
  `&StatusId` / `&str` rather than `W::Status`.
- Duplicate preconditions enforced in `knotch-kernel` — this crate
  is opt-in *preset policy*, not append-time invariants.
