---
paths:
  - "crates/**/tests/**"
  - "crates/**/src/**/*.rs"
---

# Testing placement

One concept, one home. Adding a test in the wrong layer is a
structural bug — it leaks implementation detail, duplicates
coverage, or hides invariants.

## Layer → home

| Layer | Home | Example |
|---|---|---|
| Pure type invariant | `src/<module>.rs::tests` | `Fingerprint::hash` determinism |
| Per-body precondition | `crates/knotch-kernel/tests/preconditions.rs` | `MilestoneShipped` rejects non-implementation kind |
| Repository adapter roundtrip | `crates/<adapter>/tests/<feature>.rs` | `FileRepository` reopen persists events |
| Cross-adapter parity | `crates/knotch-storage/tests/fingerprint_parity.rs` | In-memory vs file produce same fingerprint |
| Observer contract | `crates/knotch-observer/tests/<name>.rs` | cancellation stops in-flight observer |
| Reconciler semantics | `crates/knotch-reconciler/tests/<feature>.rs` | 10× idempotent replay |
| CLI surface | `crates/knotch-cli/tests/cli.rs` | `assert_cmd` golden paths |
| Hook integration | `crates/knotch-cli/tests/hook.rs` | stdin JSON → exit code + `.knotch/` state |
| Agent integration primitives | `crates/knotch-agent/tests/<name>.rs` | per-function `<W>` generic tests against `InMemoryRepository` |
| Example-driven walkthroughs | `examples/<name>/src/main.rs` | end-to-end workflow scenario, runs on CI (`examples` job) |
| Lint rule | `crates/knotch-linter/src/rules.rs::tests` + `tests/self_lint.rs` + fixture files under `tests/fixtures/` | R1/R2/R3 + self-lint |
| Proc macro compile-fail | `crates/knotch-derive/tests/ui/` + `tests/trybuild.rs` | `#[workflow]` error messages |

## Never

- Repository adapters testing per-body preconditions — the
  precondition contract lives in the kernel; adapters test
  *evaluation*, not *rules*.
- Preset crates duplicating kernel tests. Presets test their
  `WorkflowKind` impl (required phases, terminal statuses), not the
  kernel's precondition dispatch.
- `InMemoryRepository`-only tests for behavior the file-backed
  adapter must also satisfy — add parity tests instead.

## What makes a test obsolete

- The invariant it checks moved to a different layer → delete and
  add the check at the new layer.
- The test asserts a specific error *message* (vs variant) → replace
  with `matches!(err, SomeError::Variant { .. })`.
- The test encodes a workaround for a bug that was fixed at the root
  → delete.
