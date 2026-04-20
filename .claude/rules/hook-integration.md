---
paths:
  - "crates/knotch-agent/**"
  - "crates/knotch-cli/src/cmd/hook/**"
  - "plugins/knotch/hooks/**"
---

# Hook integration contract

Claude Code hooks are the deterministic enforcement surface for the
knotch event ledger. Every hook entry point is a thin wrapper around
a `knotch-agent` function. Deviations from the rules below break the
guarantee that "if a git commit succeeds, its ledger event is either
recorded or queued for reconciliation".

## Milestone opt-in policy

**A commit becomes a `MilestoneShipped` event only when its message
carries an explicit `Knotch-Milestone: <id>` git trailer.**

Non-trailer commits pass `check-commit` / `verify-commit` without
touching the ledger. This is **by design** — see
`.claude/rules/event-ownership.md` opt-in matrix for the rationale.

Recommended commit shape:

```text
feat: add SSO login flow

Describe the change in the body as usual.

Knotch-Milestone: SPEC-123
```

Agents can emit the trailer either as the last paragraph of the
first `-m` or as a second `-m` argument. Git joins multiple `-m`
values with blank lines, so both forms produce the same commit
body. `knotch-agent::commit::extract_commit_message` reassembles
every `-m` / `--message=` / `-F <file>` source via
[`shell-words`](https://crates.io/crates/shell-words).

## Exit-code contract

Claude Code only reads exit 2 as blocking. Exit 1 and every other
non-zero code are treated as non-blocking and the tool call proceeds.
`knotch-cli::hook::*` MUST map the agent functions as follows:

| `Result<HookOutput, HookError>`       | stdout                            | stderr  | exit |
|---------------------------------------|-----------------------------------|---------|------|
| `Ok(Continue)`                        | empty                             | empty   | 0    |
| `Ok(Context(s))`                      | JSON `hookSpecificOutput.additionalContext = s` | empty   | 0    |
| `Ok(UpdateInput(v))`                  | JSON `permissionDecision=allow + updatedInput=v`| empty   | 0    |
| `Ok(Ask { reason })`                  | JSON `permissionDecision=ask`                   | empty   | 0    |
| `Ok(Block { reason })`                | empty                             | `reason`| **2**|
| `Err(HookError::Orphan)`              | empty                             | empty   | 0    |
| `Err(HookError::NotAProject)`         | empty                             | empty   | 0    |
| `Err(_)` — `PreToolUse` event         | empty                             | message | **2**|
| `Err(_)` — `PostToolUse` event        | empty                             | empty   | 0 (and queued) |
| `Err(_)` — `SessionStart` / `UserPromptSubmit` / `SubagentStop` / `SessionEnd` | empty | message | 0 |

Never emit exit 1 or exit 3+. Those would let a tool call proceed
with no ledger record.

## Retry + queue (PostToolUse only)

`PostToolUse` hooks fire **after** the tool has succeeded. The
commit is already in the repository; we cannot block. On any
`Err(HookError::Repository(_))` or `HookError::Io(_)`:

1. Retry the `Repository::append` up to 3× with exponential backoff
   (50ms / 200ms / 800ms). Note that `FileRepository` already drives
   a bounded CAS retry internally (see `.claude/rules/append-flow.md`
   step 6), so this outer retry only fires for non-`LogMutated`
   errors (network, permission, lock timeout).
2. On final failure, enqueue via
   `knotch_agent::queue::enqueue_raw(queue_dir, unit, proposal_json,
   reason, &queue_config)`. The `QueueConfig` carries `max_entries`
   and `OverflowPolicy` — `Reject` (default) surfaces
   `HookError::QueueFull`, `SpillOldest` drops the lexicographically
   oldest entry to make room.
3. On `HookError::QueueFull`, fall back to the orphan log at
   `~/.knotch/orphan.log` so the event is never silently dropped —
   operators drain via `knotch reconcile` then recover the orphan
   record by hand.
4. Exit 0 unconditionally.

`SessionStart` auto-drains the queue so durable failures self-heal
on the next session. `knotch reconcile` manually drains;
`knotch reconcile --prune-stale` removes entries that no longer
deserialize into `Proposal<Knotch>` (the active workflow type).

## Active-unit resolution

Every hook entry resolves the active unit through a three-layer
chain (highest priority first):

1. `KNOTCH_UNIT` env var — explicit override, for single-shot CLI
   invocations and shell wrappers.
2. `.knotch/sessions/<session_id>.toml` — per-session pointer
   snapshotted by the `SessionStart` hook.
3. `.knotch/active.toml` — project-global pointer (written by
   `knotch unit use`).

On `ActiveUnit::NoProject` hooks exit 0 silently (knotch wasn't
used in this directory). On `Uninitialized` they log orphan and
continue without touching the ledger.

## Session lifecycle

- `SessionStart` writes `.knotch/sessions/<id>.toml` from the
  current global active unit. Later `knotch unit use` elsewhere
  does **not** disturb the running session.
- `SessionEnd` with `reason != "resume"` removes the per-session
  pointer via `active::clear_session`. `reason == "resume"` keeps
  it so the next session restart reuses the same target.
- Queue auto-drain happens on `SessionStart`. `SessionEnd` logs
  residual queue size for visibility.

## Fallback for Claude Code < v2.1.85

The `if` field on hook handlers (which scopes a hook to specific
`Bash(git commit *)` invocations) requires v2.1.85+. On older
versions the hook fires on every Bash call. Each subcommand must
re-validate `tool_input.command` and return `HookOutput::Continue`
immediately when the prefix does not match its target. Helpers:
`knotch_agent::commit::extract_commit_message`,
`knotch_agent::commit::parse_conventional`.

## Causation construction

Hook-emitted causations use `knotch_agent::causation::hook_causation`
which constructs:

- `source = Source::Hook`
- `principal = Principal::Agent { agent_id, model, harness }`
  - `agent_id` = `input.event.agent_id()` when the event carries it
    (`SubagentStop`), otherwise the session id as best-effort
    fallback.
  - `model` = `$KNOTCH_MODEL` env var, or `"unknown"`.
  - `harness` = `$KNOTCH_HARNESS` env var, or `"claude-code"`.
- `trigger = Trigger::GitHook { name }` where `name` is the
  subcommand (`"check-commit"`, `"verify-commit"`, …).
- `session = SessionId::parse(&input.session_id)` — UUID when
  possible, `Opaque(CompactString)` fallback.

The CLI entry point is the only place allowed to build a
`Causation`; `knotch-agent` functions receive it as a parameter.

## Forbidden

- Calling `Repository::append` outside a `knotch-agent` function.
- Printing anything to stdout other than the hook JSON output.
- Emitting a non-zero exit code that is not 2.
- Using `--no-verify` or any flag that bypasses the hook path.
- Reading from `$CLAUDE_ENV_FILE` to find the active unit — the env
  file is for Bash tool state, not hook state.
