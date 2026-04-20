# knotch-agent

Hook/skill integration library. Every function is `<W: WorkflowKind>`
generic, so binaries can swap presets without re-implementing the
semantics. `knotch-cli` is the reference wrapper; other consumers
plug in their own CLI and call the same functions.

@../../.claude/rules/constitution.md
@../../.claude/rules/hook-integration.md
@../../.claude/rules/event-ownership.md
@../../.claude/rules/causation.md
@../../.claude/rules/harness-decoupling.md
@../../.claude/rules/no-unsafe.md

## Module map

| Module        | Owns                                                          |
|---------------|---------------------------------------------------------------|
| `input`       | `HookInput` + `HookEvent` — decoded Claude Code hook stdin    |
| `output`      | `HookOutput` — `Continue` / `Context` / `Block` / `UpdateInput` / `Ask` |
| `error`       | `HookError` — unified error taxonomy                          |
| `causation`   | `hook_causation(input, subcommand)` — single `Causation` builder every hook uses |
| `active`      | `.knotch/active.toml` read/write, `project_root` discovery    |
| `queue`       | `.knotch/queue/*.json` per-entry reconciler queue             |
| `orphan`      | `~/.knotch/orphan.log` advisory logging                       |
| `session_start` | `SessionStart` → inject active-unit context                 |
| `commit`      | `check` / `verify` / `record_revert` — git-driven events      |
| `guard`       | `rewrite` — block destructive history-rewriting git ops       |
| `subagent`    | `SubagentStop` → `EventBody::SubagentCompleted` append        |
| `tool_call`   | `PostToolUseFailure` detector → `EventBody::ToolCallFailed` append |
| `model`       | `SessionStart.model` detector → `EventBody::ModelSwitched` append |
| `context`     | `UserPromptSubmit` — re-inject context (opt-in)               |
| `session_end` | `SessionEnd` — surface reconciler queue size                  |

## Extension recipe — add a hook

1. Add a function in the matching module, generic over `W` and
   `R: Repository<W>`. Return `Result<HookOutput, HookError>`.
2. Add a `knotch hook <subcommand>` wrapper in
   `crates/knotch-cli/src/cmd/hook/<name>.rs`.
3. Register the subcommand variant in `cmd/hook/mod.rs::HookCommand`
   and the dispatch match arm.
4. Add the hook entry to `knotch-cli::cmd::init::knotch_hook_block()`
   **and** `plugins/knotch/hooks/hooks.json`.
5. Update `.claude/rules/event-ownership.md` if the hook emits a
   new event variant.

## Milestone opt-in policy

`check-commit` / `verify-commit` only record `MilestoneShipped`
when the commit carries an explicit `Knotch-Milestone: <id>` git
trailer. A conventional-commit prefix (`feat:` / `fix:` / …) is
**not** enough — the trailer is the author's deliberate act of
naming a milestone. Without it, the hook is a silent no-op.

Why: one feature usually ships across several incremental commits.
A slug-from-description heuristic would record each as a distinct
event, inflating the log and defeating dedup. See
`commit::extract_milestone_id`.

## Do not

- Call `Repository::append` outside a `knotch-agent` function —
  hook/skill owners must be uniform (see
  `.claude/rules/event-ownership.md`).
- Use stdout for anything other than the hook JSON output.
- Rely on `$CLAUDE_ENV_FILE` for state — hooks don't see env vars
  written by prior hooks.
- Emit exit codes other than 0 or 2 — Claude Code treats all other
  codes as non-blocking, breaking ledger integrity.
