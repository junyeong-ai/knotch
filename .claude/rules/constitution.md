---
paths:
  - "**"
---

# Knotch Constitution

Immutable principles. Every design decision traces to one of them
or requires a superseding entry here.

## I. Event log is the only truth

Projections derive state. No cache may claim authority.

Canonical sites:
`crates/knotch-kernel/src/log.rs` (`Log<W>`),
`crates/knotch-kernel/src/project.rs` (all built-in projections).

## II. Single writer per unit

Only `Repository::append` writes the log. Direct writes to
`log.jsonl` / `.resume-cache.json` are blocked by `knotch-linter`
rule **R1**. Violations are errors, not warnings.

See `crates/knotch-linter/src/rules.rs` (DirectLogWriteRule).

## III. Idempotence by construction

Every proposal carries a content-addressed `Fingerprint` —
`BLAKE3(salt || JCS(dedup tuple))`. Replayed proposals land in
`AppendReport::rejected` with reason `"duplicate"`, never double-append.

Canonical: `crates/knotch-kernel/src/fingerprint.rs::fingerprint_proposal`.
Dedup tuple is closed (not user-swappable) — see
@.claude/rules/fingerprint.md.

## IV. Purity boundary

`knotch-kernel` and `knotch-proto` perform zero I/O. Enforced by
`knotch-linter` rule **R3** (KernelNoIo).

Bans: `std::fs`, `std::net`, `tokio::fs`, `tokio::net`, `gix` in those
two crates.

## V. Hexagonal ports-and-adapters

`Repository`, `Storage`, `Lock`, `Vcs`, `Observer` are traits in the
kernel / proto layer. Concrete adapters live in sibling crates.

Users swap adapters without forking kernel types.

## VI. Clean from zero

No backcompat shims. No deprecated aliases. No "legacy mode". Delete
old code in the same change as the replacement. Breaking changes
bump the major.

## VII. Automated compliance

Any rule that can be a CI gate is. **No rule is enforced by review
alone.** Active gates:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo knotch-linter` (R1, R2, R3)
- `cargo public-api --diff-against docs/public_api/<crate>.baseline`
- `cargo semver-checks`
- `cargo deny`

## VIII. Agent-first observability

Every event carries sufficient attribution for AI-driven
workflows: `Source`, `SessionId`, `agent_id`, and a typed
`Trigger` (CLI command / git hook / tool invocation / reconciler
observer). Model attribution lives on dedicated
`EventBody::ModelSwitched` events so mid-stream model changes
are faithfully recorded.

See @.claude/rules/causation.md.

## IX. Determinism

Given identical inputs, observers produce identical proposals; the
reconciler sorts deterministically; replay produces identical logs.

Sort key: `(observer_name, EventBody::kind_ordinal, kind_tag)` —
see `crates/knotch-reconciler/src/lib.rs::kind_tag`.
