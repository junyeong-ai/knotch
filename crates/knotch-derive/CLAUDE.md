# knotch-derive

Proc macros that generate the boilerplate every adopter's
`WorkflowKind` impl needs.

@../../.claude/rules/no-unsafe.md

## Macros

| Macro | Role |
|---|---|
| `#[derive(PhaseKind)]` | Emits `PhaseKind` impl for an enum whose variants represent ordered phases. Requires unit variants in canonical order. Generates `id`, `required_artifacts` (empty), `next` (walks enum order), `is_skippable` (returns `false` — override by hand-writing the trait when you need custom skip rules). |
| `#[derive(MilestoneKind)]` | Emits `MilestoneKind` for (a) enums with unit variants or (b) newtype tuple structs wrapping a single `CompactString`. |
| `#[derive(GateKind)]` | Same shape as `MilestoneKind` — enum of unit variants or newtype string. |
| `#[workflow(...)]` | Composes the three derives above plus the `WorkflowKind` trait impl from a single attribute on the marker struct. |

## Extension recipe — add a new derive

1. Add the macro in `src/lib.rs` with a `#[proc_macro_derive]` attribute.
2. Put the token-generation logic in a sibling module; `lib.rs` stays a thin dispatcher.
3. Cover every rejection path with a UI test in `tests/ui/` — `trybuild` renders them at `cargo test`.
4. Document the generated impl in the doc comment on the `#[proc_macro_derive]` function, not in `lib.rs`'s top.

## Do not

- Emit code that calls a function from a specific adopter crate — derives must remain workflow-agnostic.
- Wrap the generated impl in `unsafe` — `#![forbid(unsafe_code)]` at the workspace level catches it.
- Add compile-time policy (e.g. "reject a phase named `done`") — policy is a runtime concern for the `WorkflowKind::is_skippable` check and its siblings.
