# knotch-workflow

Ships two concrete `WorkflowKind` paths plus the runtime machinery
they share: the typed canonical `Knotch` workflow for Rust-first
adopters, and `ConfigWorkflow` for zero-Rust adopters who declare
their shape in `knotch.toml`. Both paths go through the same
`knotch-kernel` invariants (fingerprint dedup, optimistic CAS,
terminal immutability, kernel-enforced gate ordering).

@../../.claude/rules/no-unsafe.md

## Module map

| Module | Owns |
|---|---|
| `knotch` | `Knotch` marker, `KnotchPhase`, `KnotchGate` (kebab-case serde), `TaskId`, `build_repository` helper — the canonical typed workflow |
| `config` | `ConfigWorkflow` + `WorkflowSpec` / `PhaseSpec` / `GateSpec` + `ConfigError` — TOML-loaded runtime workflow. `required_phases` accepts arbitrary scope keys (tiny / standard / epic / any adopter-chosen tag); `default_scope` must name one of them. `CANONICAL_TOML` ships the canonical shape for `knotch init` to stamp into `knotch.toml` |
| `dynamic` | `DynamicPhase` / `DynamicGate` / `DynamicMilestone` / `DynamicExtension` — all `#[serde(transparent)]` newtypes over `CompactString` / `serde_json::Value`; used by `ConfigWorkflow` |
| `ordering` | `PhaseOrdering` — compact declarative graph; `validate_ordering` runs acyclicity + uniqueness checks |
| `skip` | `SkipPolicy` — reusable predicate describing which `SkipKind` values a phase accepts |

## When to pick typed enum vs config

- Compile-time exhaustiveness + typed `Extension` payload matters →
  implement `WorkflowKind` on a marker struct, derive `PhaseKind` /
  `MilestoneKind` / `GateKind` on the associated enums. Shipped
  `knotch` CLI does NOT use this path — it's for third-party
  harnesses and Rust-first adopter binaries.
- Zero-Rust adoption, operator-editable workflow shape → let the
  shipped `knotch` CLI load `knotch.toml [workflow]` into
  `ConfigWorkflow`. This is the default binding.

Both paths produce byte-identical fingerprints for the canonical
shape — verified by `tests/canonical_parity.rs`.

## Extension recipe

**Add a new shared policy helper (applies to both static + dynamic kinds):**

1. Add a module under `src/` with a single focused trait or struct.
2. Export it from `lib.rs` via the top-level `pub use`.
3. Write unit tests next to the module; structural invariants go
   in `crates/knotch-kernel/tests/` per `@.claude/rules/testing.md`.

## Do not

- Re-implement `validate_ordering` — derive macros and dynamic
  types both call the same helper so invariants stay identical.
- Introduce adopter-specific logic here — this crate ships the
  canonical `Knotch` workflow, `ConfigWorkflow` as the
  config-driven generic, plus policy-free runtime helpers.
  Adopter-specific shapes fork a case study in
  `examples/workflow-*-case-study/` (Rust path) or edit
  `knotch.toml [workflow]` (config path).
