# knotch-linter

AST-based lint rules. Ships as a library plus the
`cargo-knotch-linter` cargo subcommand. Rules enforce
constitutional invariants that cannot be expressed in rustc or
clippy.

@../../.claude/rules/constitution.md
@../../.claude/rules/no-unsafe.md

## Rules shipped

| Id | Rule | Enforces |
|---|---|---|
| R1 | `DirectLogWriteRule` | Only `knotch-storage` may write `log.jsonl` / `.resume-cache.json` (constitution §II) |
| R2 | `ForbiddenNameRule` | Reject identifiers ending in `Helper` / `Util` / `Manager` / `Handler` / `Processor` / `Impl` |
| R3 | `KernelNoIoRule` | `knotch-kernel` and `knotch-proto` may not use `std::fs`, `std::net`, `tokio::fs`, `tokio::net`, or `gix` (constitution §IV) |

## Extension recipe — add a rule

1. Add a `pub struct Rule...` in `src/rules.rs`, impl `Rule`
   (trait in `lib.rs`). Define `id() -> RuleId::Rn`, a description,
   and `check(ctx, file) -> Vec<Violation>`.
2. Register the instance in `lib::default_rules()` so the binary
   picks it up.
3. Add a fixture pair (violating + clean) under
   `tests/fixtures/` and a golden-file test in
   `tests/self_lint.rs` + the rule-specific test module in
   `src/rules.rs::tests`.
4. Update the rule table in this file **and** the CI gate list in
   `.claude/rules/constitution.md` §VII.

## Do not

- Let a rule read outside the `syn::File` AST it is handed —
  single-file evaluation keeps runs parallel-safe.
- Silently degrade on parse failure — surface it as a `Violation`
  with `Severity::Error` so CI fails visibly.
- Downgrade an R-rule to a warning — every rule that ships here is
  an error per constitution §VII ("no rule enforced by review
  alone").
