# workflow-vibe-case-study

Reference fork of the canonical `knotch_workflow::Knotch` showing
an AI-pair-programming workflow — phases `Intent → Explore →
Implement → Verify` tuned for agent-driven work with short-lived
free-form milestones. Copy this source into your adopter repo as
a starting point when your shape matches.

@../../.claude/rules/constitution.md
@../../.claude/rules/causation.md
@../../.claude/rules/preconditions.md

## Surface

| Type | Role |
|---|---|
| `Vibe` | The `WorkflowKind` marker |
| `VibePhase` | `Intent / Explore / Implement / Verify` |
| `TaskId` | Milestone — free-form short id coined per unit |
| `VibeGate` | `IntentClear / Handoff` |
| `Session` | `Causation` factory: `Session::new(agent, model).tool(tool, call_id)` |
| `summary_for_llm` | Budget-capped markdown summary for prompt injection |
| `SummaryBudget` | Char-count cap for `summary_for_llm` |

Terminal statuses: `archived`, `abandoned`, `handed_off`.

## Extension recipe

**Add a new gate:**

1. Extend `VibeGate` with the new variant.
2. Vibe has no G-ladder order check — gates gate-check is
   per-skill. Add skill-level guidance if the new gate has
   prerequisites.

**Add a new projection helper (activity, …):**

1. Place the pure fn in `lib.rs`; take `&Log<Vibe>` and return a
   plain value (no `Result` unless the projection can fail on
   well-formed input).
2. Reference from `summary_for_llm` if it belongs in the
   LLM-facing summary; otherwise expose via `pub use`.

## Do not

- Widen `Session::new` signatures — the factory is the stable
  agent-facing entry. Add chainable `with_*` methods instead.
- Record a `MilestoneShipped` from the preset — vibe milestones
  still go through `knotch-agent::commit::verify` like every
  other preset.
