# Adopter migration playbook

knotch is a library for AI-agent-driven workflow state. Adopters
replace their internal state-management code by following the
phased pattern below. Per-project migration plans stay in the
adopter's own repository — this document is the universal
playbook every migration must follow.

## Universal rules

Non-negotiable for every adopter. Plans may add detail but not
relax these.

### Integration: CLI subprocess, never in-process bindings

knotch's consumption surface is the `knotch` binary. Python,
TypeScript, or any other host language shells out and parses
JSON. No PyO3, napi, wasm, or equivalent.

**Why**: universality. Every binding is a new maintenance surface
against knotch's public API. The binary is the one interface all
hosts share. Subprocess startup is tens of milliseconds — well
below lifecycle-event cadence. The hypothetical "hot-loop
projection read" justification for in-process bindings does not
match any adopter flow today; if reporting ever demands that
shape, the answer is a cached materialised view fed by `knotch
show`, not a language binding.

### Phased pattern

Each phase exits on measurable criteria. No phase starts until
the prior phase exits cleanly.

1. **Inventory.** Classify every call site of the internal state
   layer. Separate adopter-specific logic (stays native) from
   ledger work (moves to knotch).
2. **Pilot.** One low-traffic unit runs end-to-end through
   knotch in shadow mode. The internal system keeps writing;
   knotch writes in parallel. Diff outputs byte-for-byte for a
   defined bake period before cutover.
3. **Cutover.** Kill the dual-write. The knotch log becomes the
   single source of truth. Delete the adopter-side reconciler,
   observer, and log writer.
4. **Hook / skill install.** Run `knotch init --with-hooks` or
   install `plugins/knotch/`. Replace adopter skill bodies with
   thin wrappers around `/knotch-*`. Model attribution is
   zero-config — the `load-context` hook reads
   `SessionStart.model` directly from every Claude Code payload.
5. **Cleanup.** Delete code that only existed to support the
   internal layer. `@`-import knotch's rules from the adopter's
   `.claude/rules/` tree so principles load automatically.

### Data migration

Internal snapshot → `log.jsonl` via per-event replay through the
CLI. The fingerprint-dedup invariant
(`@../../.claude/rules/fingerprint.md`) makes the replay script
idempotent; crash / re-run never double-appends. Stamp migration
events with `--causation-source migration` so downstream queries
can filter.

### Rollback

- Pre-cutover: delete `.knotch/`, keep the internal snapshot.
  Zero cost.
- Post-cutover: regenerate the snapshot from `knotch show
  --format json`, or `git revert` the deletion commit.
- Mid-migration: every plan defines explicit rollback at each
  phase exit. The longest risky window is the phase that kills
  the dual-write — bake time before progressing is load-bearing.

### Workflow fork or canonical reuse

Adopters whose phase / gate / milestone vocabulary matches the
canonical `knotch_workflow::Knotch` reuse it directly. Adopters
whose vocabulary diverges fork one of the reference case studies
in `examples/workflow-*-case-study/` as a starting point. The
choice is irreversible-per-log — a forked workflow's
`SCHEMA_VERSION` namespaces its fingerprint salt, so two
workflows over the same storage root cannot collide.

Prefer reuse. Fork only when a gate or phase identifier must
differ — identifiers are serialised into the log and bumping
them requires a `SchemaMigrator` (see
`crates/knotch-proto/CLAUDE.md`).

## knotch's role

- Ship binaries + library + skills.
- Maintain `CHANGELOG.md` with adopter-visible breaking notes.
- Publish baselines under `docs/public_api/*.baseline` so each
  adopter can audit API surface at its pinned version.
- Accept bug reports and invariant clarifications filed against
  knotch itself; refuse feature requests that would widen the
  public API for a single adopter (see
  `@../../.claude/rules/governance.md` §"Four-step PR rubric").

## What knotch does not ship for migrations

Per `@../../.claude/rules/governance.md`:

- Adopter-specific migration plans — those live in the adopter's
  own repository.
- Adopter-specific rules or skills — `.claude/rules/` here ships
  only ledger-structural rules.
- In-process language bindings (see universal rule above).
- Bulk-import subcommands or one-shot migration utilities —
  adopter migration is a per-release concern of each adopter,
  not a permanent public-API surface.
