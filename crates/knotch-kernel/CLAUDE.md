# knotch-kernel

The purity core. **Zero I/O.** Type vocabulary + invariant contracts
for every downstream crate.

@../../.claude/rules/constitution.md
@../../.claude/rules/causation.md
@../../.claude/rules/preconditions.md
@../../.claude/rules/fingerprint.md
@../../.claude/rules/no-unsafe.md

## Module map

| Module | Owns |
|---|---|
| `workflow` | `WorkflowKind` + `PhaseKind` / `MilestoneKind` / `GateKind` / `ExtensionKind` |
| `event` | `Event<W>`, `EventBody<W>`, `CommitStatus`, `CommitKind`, `RetryAnchor`, `Proposal<W>`, `AppendMode`, `AppendReport<W>`, `EventBody::check_precondition`, `kind_tag`, `kind_ordinal` |
| `causation` | `Causation`, `Principal`, `Source`, `Trigger`, `Cost`, `Person`, `AgentId`, `ModelId`, `Harness`, `SessionId`, `TraceId` + `Sensitive` marker |
| `repository` | `Repository<W>` port, `ResumeCache`, `CacheError`, `SubscribeEvent`, `SubscribeMode` |
| `log` | `Log<W>` (immutable snapshot), `LogError`, `try_from_events` (validated), `from_events` (`#[doc(hidden)]`, adapter-only) |
| `project` | built-in pure projections: `current_phase`, `last_completed_phase`, `current_status`, `shipped_milestones`, `effective_events`, `total_cost`, `cost_by_phase`, `cost_by_milestone`, `subagents`, `model_timeline`, `tool_call_timeline` |
| `precondition` | `AppendContext<'a, W>`, `VerifyCommit`, `ArtifactCheck` |
| `fingerprint` | `Fingerprint`, `fingerprint_proposal`, `fingerprint_event` — closed, JCS-canonical |
| `scope` / `status` / `rationale` / `id` / `time` / `error` | primitives |

## Extension recipe

**Add a new `EventBody` variant:**
1. Extend `event.rs::EventBody<W>` — it is `#[non_exhaustive]` so
   this is a minor-version change for downstream.
2. Extend `event.rs::EventBody::kind_tag` + `kind_ordinal` — the
   match is in-crate and compilation enforces completeness.
3. Extend `event.rs::EventBody::check_precondition` — write the
   invariant and cite the corresponding `PreconditionError` variant.
   **Log iteration rule**: use `effective_events(ctx.log)` whenever
   the check reasons about current *state* (e.g. "was this
   milestone shipped", "is this gate's prerequisite present"). Use
   the raw `ctx.log.events()` only when the check reasons about
   *append history* — `UnitCreated` (log-is-empty),
   `EventSuperseded` (dedup against prior supersedes). Mixing
   these up silently admits events that should be refused.
4. Extend `error.rs::PreconditionError` if the check surfaces a new
   failure mode.
5. Add tests in `tests/preconditions.rs` (pass + fail per variant).
6. Add rows to `.claude/rules/event-ownership.md` (Owner table +
   Opt-in matrix) and `.claude/rules/preconditions.md`
   (Variant → check table). Enforced by `cargo xtask docs-lint` —
   a missing row fails CI.
7. Wire a canonical emitter — CLI subcommand, hook, skill,
   reconciler observer, or `knotch-agent` library helper (for
   harness-wired variants; see
   `.claude/rules/harness-decoupling.md`).

**Add a new `WorkflowKind` associated-type convention:**
1. Extend `workflow.rs::WorkflowKind` with a default method.
2. Let presets override.
3. Reference from `EventBody::check_precondition` only if the new
   knob affects admissibility.

## Do not

- Import `std::fs`, `std::net`, `tokio::fs`, `tokio::net`, or
  `gix` — blocked by `knotch-linter` R3.
- Add a user-swappable `Fingerprinter` trait — fingerprint is closed
  by design; see @../../.claude/rules/fingerprint.md.
- Re-derive `Log` validation in adapters — use
  `Log::try_from_events` if you need safety, `from_events` if
  you know the sequence is already canonical.
