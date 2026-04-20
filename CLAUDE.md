# Knotch

Git-correlated event-sourced workflow state, built **for AI agents**.
Agents `append` events, read `projections`, and `subscribe` to live
streams — the log is the sole truth.

## Architecture

Hexagonal ports-and-adapters. `knotch-kernel` is I/O-free (enforced
by `knotch-linter` R3). `knotch-agent` is the hook/skill integration
library — every Claude Code (or compatible) harness wraps its
functions; `knotch-cli`'s `hook` subcommand is the reference
wrapper, third-party harnesses plug in their own CLI against the
same library.

Per-crate details load on demand — see the navigation table below
rather than spelling each crate's surface here.

## Commands

```bash
cargo xtask ci            # fmt + clippy + knotch-linter + nextest + deny + machete
cargo xtask docs-lint     # verify .claude/rules/ file:line citations
cargo xtask public-api    # regenerate docs/public_api/*.baseline (nightly)
cargo xtask plugin-sync   # mirror .claude/skills/ → plugins/knotch/skills/
```

## Where things live

| Task | Read |
|---|---|
| Interact with knotch as an agent | @.claude/skills/knotch-query/SKILL.md, @.claude/skills/knotch-mark/SKILL.md, @.claude/skills/knotch-gate/SKILL.md, @.claude/skills/knotch-transition/SKILL.md |
| Understand constraints | @.claude/rules/constitution.md |
| Change `knotch-kernel` | crates/knotch-kernel/CLAUDE.md |
| Change `knotch-storage` / `-lock` / `-vcs` | crates/knotch-storage/CLAUDE.md, crates/knotch-lock/CLAUDE.md, crates/knotch-vcs/CLAUDE.md |
| Add an observer | crates/knotch-observer/CLAUDE.md |
| Add a reconciler feature | crates/knotch-reconciler/CLAUDE.md |
| Change hook/skill behavior | @.claude/rules/hook-integration.md, @.claude/rules/event-ownership.md, @.claude/rules/harness-decoupling.md, crates/knotch-agent/CLAUDE.md |
| Tune the canonical workflow (phases, gates, statuses) | crates/knotch-workflow/CLAUDE.md |
| Fork a workflow for a different shape | examples/workflow-spec-driven-case-study/, examples/workflow-vibe-case-study/ |
| Add a lint rule | crates/knotch-linter/CLAUDE.md |
| Add a CLI subcommand | crates/knotch-cli/CLAUDE.md |
| Add a schema / lifecycle FSM | crates/knotch-schema/CLAUDE.md |
| Sync Markdown frontmatter to ledger status | crates/knotch-frontmatter/CLAUDE.md |
| Record ADR-style lifecycle events | crates/knotch-adr/CLAUDE.md |
| Write a cross-unit query | crates/knotch-query/CLAUDE.md |
| Add a tracing attribute / OTel bridge | crates/knotch-tracing/CLAUDE.md |
| Add a proc-macro derive | crates/knotch-derive/CLAUDE.md |
| Reach for an in-memory fake in tests | crates/knotch-testing/CLAUDE.md |
| Write a test | @.claude/rules/testing.md (one layer, one home) |
| Wire hooks into Claude Code | `knotch init --with-hooks` (merges into `.claude/settings.json`) or install the `plugins/knotch/` bundle |
| Migrate an adopter onto knotch | docs/migrations/README.md (universal playbook + links to per-adopter plans) |

The `paths:` frontmatter on each `.claude/rules/*.md` declares
which files the rule applies to. Per-crate `CLAUDE.md` import the
rules that govern their surface via `@..` paths.

## Constitution

@.claude/rules/constitution.md

## Scope contract

@.claude/rules/governance.md

## Working style

- **Evidence-based.** Every claim cites `crates/.../file.rs:line`.
  Backticked citations are verified by `cargo xtask docs-lint`.
- **Root-cause over patch.** No TODO / FIXME / backcompat shims /
  deprecated aliases. Fix at the structural origin.
- **Single-bound generic.** One `W: WorkflowKind` trait parameter
  threads through every generic API — never four.

## Commit conventions

Commits follow [Conventional Commits](https://www.conventionalcommits.org)
(`feat:` / `fix:` / `refactor:` / `perf:` / `docs:` / `chore:` / …).

**Milestones are opt-in.** Commits that finalize a milestone
carry a `Knotch-Milestone: <id>` git trailer. Full contract +
trailer extraction details live in
`crates/knotch-agent/CLAUDE.md`.

Phase events (`PhaseCompleted` / `PhaseSkipped`) do not go
through commits — use `/knotch-mark`.

## Runtime + release

- Rust channel pin: `rust-toolchain.toml`. External crate
  versions: root `Cargo.toml` `[workspace.dependencies]`. Bump
  both together.
- Public API surface is diffed against
  `docs/public_api/<crate>.baseline` in CI. Every public-API
  change regenerates the baseline in the same commit;
  `cargo-semver-checks` enforces semver.
