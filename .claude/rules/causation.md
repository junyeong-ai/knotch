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
Causation { source, principal, session, trace, trigger, parent_event, cost }
```

`#[non_exhaustive]` — peer crates cannot use `Causation { ... }`
syntax; call `Causation::new(source, principal, trigger)` then chain
`with_session / with_trace / with_parent_event / with_cost`.

## Principal

| Variant | When |
|---|---|
| `Human { person: Person }` | CLI operator or reviewer (rare in agent-only flow) |
| `Agent { agent_id, model, harness }` | LLM turn — this is the common case |
| `System { service }` | CI, observer, reconciler, knotch CLI |

`Person` and `AgentId` implement `Sensitive` — `knotch-tracing`
subscribers hash them to BLAKE3-16 prefix. Plain public info
(`ModelId`, `Harness`) is emitted verbatim.

## Trigger

| Variant | When |
|---|---|
| `Manual` | fallback; prefer something more specific |
| `Command { name }` | CLI subcommand |
| `GitHook { name }` | pre-commit / post-commit |
| `ToolInvocation { tool, call_id }` | agent tool call — use this for every event an agent emits |
| `Observer { name }` | reconciler-driven; observer name only |

All variants are **struct-form** — tuple variants serialize
positionally under RFC 8785 JCS, which would break fingerprint
stability the first time a field were added.

## Cost

`Cost::new(usd: Option<Decimal>, tokens_in: u32, tokens_out: u32)`
— `#[non_exhaustive]`, so never struct-literal. Aggregated by
`knotch_kernel::project::total_cost`. Adopter workflows may add
per-workflow roll-ups (e.g. `total_usd`) in their own
`WorkflowKind` crate — see
`examples/workflow-vibe-case-study/src/lib.rs` for a reference
implementation.

### How agents populate `Cost`

The canonical pattern is **stamp cost at the same place you build
`Causation`**. Three concrete approaches:

1. **Hook surface (Claude Code)** — `knotch-agent`'s
   `hook_causation(&input, subcommand)` constructs the envelope
   without cost; hooks that know their LLM spend chain
   `.with_cost(Cost::new(usd, tokens_in, tokens_out))` on the
   returned value before passing it to the agent helper.
2. **Agent-driven CLI** — build a `Session` in the harness
   (see `examples/workflow-vibe-case-study/src/lib.rs::Session`),
   then `session.tool(name, call_id).with_cost(cost)`. The
   `Session` carries agent / model / harness identity so cost
   attribution follows the agent automatically.
3. **Skill / CLI path** — when a human operator runs a command,
   there's no LLM cost to attribute; leave `Causation::cost ==
   None`. `total_cost` skips `None` entries cleanly.

**Never** pass zero-cost placeholders (`Cost::new(None, 0, 0)`)
when the real cost is unknown — the projection can't distinguish
"truly zero-cost human action" from "we forgot to stamp". Use
`None` whenever cost is unavailable.

`knotch-tracing` writes the same `Cost` fields as structured
span attributes so external observability (OTel, Prometheus) can
join agent spans to the ledger on the same identifiers.

## Session helper

Adopter workflows typically expose a `Session::new(agent, model,
harness) → .tool(tool, call_id) → Causation` builder rather than
constructing `Principal::Agent` by hand. The canonical
`Knotch` workflow does not ship one — agents either call
`hook_causation` (from `knotch-agent`, used by every shipped
hook) or construct `Causation::new(...)` directly. See
`examples/workflow-vibe-case-study/` for a `Session` reference.

## Reasons this is a rule

- Agent audit trail must survive a reboot + replay.
- Cost must be attributable at event level — not at span level —
  because spans are transient and logs are the sole truth (§I).
- Sensitive fields must not leak to stdout or tracing sinks; the
  marker trait is the only enforcement.
