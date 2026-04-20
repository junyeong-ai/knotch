---
paths:
  - "crates/knotch-agent/**"
  - "crates/knotch-cli/src/cmd/**"
  - ".claude/skills/**"
  - "plugins/knotch/skills/**"
---

# Event ownership

Each `EventBody<W>` variant has exactly **one canonical emitter**
and a **single opt-in surface**. Duplication across layers causes
double-appends (dedup protects the log but wastes proposals) or
gaps (a variant that "everyone could emit" is often emitted by
nobody). Both tables below are authoritative; new layers propose
additions here before shipping.

## Owner table

| `EventBody` variant   | Owner      | Emitter                                      |
|-----------------------|------------|----------------------------------------------|
| `UnitCreated`         | CLI        | `knotch unit init <id>`                      |
| `PhaseCompleted`      | Skill      | `/knotch-mark completed <phase>`             |
| `PhaseSkipped`        | Skill      | `/knotch-mark skipped <phase> --reason …`    |
| `MilestoneShipped`    | Hook       | `knotch hook verify-commit` (PostToolUse)    |
| `MilestoneVerified`   | Reconciler | `PendingCommitObserver`                      |
| `MilestoneReverted`   | Hook       | `knotch hook record-revert` (PostToolUse)    |
| `GateRecorded`        | Skill      | `/knotch-gate <gate-id> <decision> <text>`   |
| `StatusTransitioned`  | Skill      | `/knotch-transition <target> [--forced …]`   |
| `ReconcileFailed`     | Reconciler | Reconciler itself on observer failure        |
| `ReconcileRecovered`  | Reconciler | Reconciler itself on recovery                |
| `EventSuperseded`     | CLI        | `knotch supersede <event-id> <rationale>`    |
| `SubagentCompleted`   | Hook       | `knotch hook record-subagent` (SubagentStop) |
| `ToolCallFailed`      | Hook       | `knotch hook record-tool-failure` (PostToolUse)         |
| `ModelSwitched`       | Hook       | `knotch hook load-context` (SessionStart — detects env vs log) |
| `ApprovalRecorded`    | CLI        | `knotch approve <unit> <event-id> <decision> <rationale>`|

## Opt-in matrix

Some emitters are **deterministic** (always emit when triggered);
others are **opt-in** and require an explicit signal. Conflating
the two is the main source of false-positive events.

| Variant              | Emission mode | Trigger                                                |
|----------------------|---------------|--------------------------------------------------------|
| `UnitCreated`        | Explicit      | Operator runs `knotch unit init`                       |
| `PhaseCompleted`     | Explicit      | Agent invokes `/knotch-mark completed`                 |
| `PhaseSkipped`       | Explicit      | Agent invokes `/knotch-mark skipped`                   |
| `MilestoneShipped`   | **Opt-in**    | Commit carries `Knotch-Milestone: <id>` git trailer    |
| `MilestoneVerified`  | Automatic     | VCS newly sees a `Pending` commit                      |
| `MilestoneReverted`  | Automatic     | `git revert` whose original has a matching milestone   |
| `GateRecorded`       | Explicit      | Agent invokes `/knotch-gate`                           |
| `StatusTransitioned` | Explicit      | Agent invokes `/knotch-transition`                     |
| `ReconcileFailed`    | Automatic     | Reconciler observer returns error past retry anchor    |
| `ReconcileRecovered` | Automatic     | Reconciler observer succeeds after a prior failure     |
| `EventSuperseded`    | Explicit      | Operator runs `knotch supersede`                       |
| `SubagentCompleted`  | Automatic     | Claude Code fires `SubagentStop` for a delegated task  |
| `ToolCallFailed`     | Automatic     | Claude Code PostToolUse `tool_response.error` or non-zero Bash `exit_code` |
| `ModelSwitched`      | Automatic     | `load-context` hook compares `$KNOTCH_MODEL` to `model_timeline` at SessionStart |
| `ApprovalRecorded`   | Explicit      | Reviewer runs `knotch approve`                         |

**Opt-in rationale** (`MilestoneShipped`): one feature usually
lands across several incremental commits ("start X", "polish X",
"test X"). A slug-from-description heuristic would record each as
a distinct event, inflating the log and breaking dedup. The
trailer forces the author to **name** a milestone exactly once on
whichever commit finalizes it. See
`crates/knotch-agent/src/commit.rs::extract_milestone_id`.

## Why these assignments

- **Hook** — deterministic enforcement. Git commands are the only
  triggers knotch can intercept before the action completes.
- **Skill** — agent judgment. The agent decides when a phase is
  complete, when a gate passes, when status transitions. Not
  derivable from tool calls alone.
- **CLI** — human-driven, deliberate. Unit creation, supersede,
  and human approval are rare, deliberate operations; an explicit
  command prevents accidents.
- **Reconciler** — passive observation. Reads the world and emits
  whatever external state implies; never tied to a single tool call.
- **Agent lib** — harness-wired events (tool-call failures, model
  switches) for which Claude Code exposes no uniform hook trigger.
  The library ships the append helper; each harness wires its own
  detector. See `.claude/rules/harness-decoupling.md`.

## Forbidden overlaps

- Hooks emitting `PhaseCompleted` from a file-edit observation —
  file edits are too granular. Phase boundaries are the agent's call.
- Skills emitting `MilestoneShipped` from an agent self-report —
  milestones are tied to commits; commits go through the hook.
- CLI emitting `GateRecorded` — gates carry long-form rationale
  that only the agent's conversation context produces.
- Reconciler emitting `UnitCreated` — ghost units (created by
  inference) hide intent.

## Event-ownership changes

Adding a new `EventBody` variant:

1. Update `crates/knotch-kernel/src/event.rs`.
2. Update both tables above with the chosen owner **and** emission
   mode. If it's opt-in, name the opt-in signal. **Enforced by
   `cargo xtask docs-lint`** — every variant must have a row in
   both tables.
3. Add the emitter (hook subcommand, skill, CLI subcommand,
   reconciler observer, or `knotch-agent` library helper).
4. Update `.claude/rules/preconditions.md` Variant → check table
   (also enforced by `cargo xtask docs-lint`).
5. Update `.claude/rules/hook-integration.md` if a new hook is
   introduced.
