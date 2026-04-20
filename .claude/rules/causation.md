---
paths:
  - "crates/knotch-kernel/src/causation.rs"
  - "crates/knotch-tracing/**"
  - "crates/knotch-workflow/**"
  - "crates/knotch-observer/**"
  - "examples/workflow-vibe-case-study/**"
---

# Causation attribution

Every `Event<W>` carries a `Causation`. Agents must build it through
constructors, never struct literals.

## Shape (kernel-owned)

```rust
Causation { source, session, agent_id, trigger }
```

`#[non_exhaustive]` — peer crates cannot use `Causation { ... }`
syntax; call `Causation::new(source, trigger)` then chain
`with_session` / `with_agent_id`.

## Source

| Variant | When |
|---|---|
| `Cli` | `knotch` CLI subcommand (operator-driven or scripted) |
| `Hook` | Claude Code hook dispatch (`knotch hook <name>`) |
| `Observer` | Reconciler observer pass |

Three unit variants. Authorship detail (which specific subagent,
which observer, which CLI subcommand) lives in `agent_id` +
`Trigger`, not in additional `Source` variants.

## agent_id

`Option<AgentId>`. Populated when Claude Code surfaces a subagent
id in the hook payload (envelope-level `agent_id` on every
event fired inside a subagent scope, plus `SubagentStop`'s
duplicate in-variant field). Main-session hooks, CLI operators,
and observer-driven events leave it `None`.

Never synthesise an `agent_id` from the session id — the two
identify different things and conflating them breaks downstream
"what has this subagent done?" queries.

## Trigger

| Variant | When |
|---|---|
| `Command { name }` | CLI subcommand or shell invocation (including test fixtures — use `name: "test"` or a more specific tag) |
| `GitHook { name }` | pre-commit / post-commit hook dispatch |
| `ToolInvocation { tool, call_id }` | agent tool call — use this for every event an agent emits |
| `Observer { name }` | reconciler-driven; observer name only |

All variants are **struct-form** — tuple variants serialize
positionally under RFC 8785 JCS, which would break fingerprint
stability the first time a field were added.

## Model attribution lives on events, not on causation

Model identifiers (`claude-opus-4-7`, `claude-sonnet-4-6`, …) are
recorded by `EventBody::ModelSwitched` events, not by a
`Causation.model` field. Reason: the model can change within a
session (`/model` in Claude Code, cross-harness migrations), and
stamping the current model on every causation would require
stale-copy bookkeeping at every emit site.

Consumers that want "which model produced event X" read the
effective [`model_timeline`](../../crates/knotch-kernel/src/project.rs)
up to event X — one `ModelSwitched` event anchors every
subsequent event in the timeline.

The `knotch hook load-context` subcommand reads
`SessionStart.model` from every hook payload and appends a
`ModelSwitched` event when it differs from the last one
recorded. This covers startup, resume, `/clear`, and `/compact`
transitions — every lifecycle point where Claude Code re-fires
`SessionStart`.

## Session helper

Adopter workflows typically expose a
`Session::new(agent, model) → .tool(tool, call_id) → Causation`
builder rather than constructing `Causation` by hand. The
canonical `Knotch` workflow does not ship one — agents either
call `hook_causation` (from `knotch-agent`, used by every shipped
hook) or construct `Causation::new(...)` directly. See
`examples/workflow-vibe-case-study/` for a `Session` reference.

## Reasons this is a rule

- Agent audit trail must survive a reboot + replay.
- Source / Trigger splits remain orthogonal so new subagent
  vocabularies, harnesses, or observer names never require
  kernel enum changes.
- `agent_id` at the top level (not nested inside Principal or
  Trigger) keeps query predicates cheap — filtering by subagent
  is a single `causation.agent_id == Some(want)` check.
