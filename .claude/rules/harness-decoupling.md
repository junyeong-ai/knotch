---
paths:
  - "crates/knotch-agent/**"
  - "crates/knotch-cli/src/cmd/hook/**"
---

# Harness decoupling

Claude Code — and any future adapter harness (Cursor, Aider,
custom) — evolves its hook API independently of knotch. The ledger
domain model must not couple to that evolution. This rule pins the
policy so speculative typed-enum proposals are rejected without
re-running the whole governance debate each time.

## Policy

Hook envelope fields hold **domain-opaque values as open strings**
(`Option<CompactString>`) unless **both** of the following hold:

1. A knotch-owned projection or kernel precondition explicitly
   branches on the value (not adopter code — adopters can layer
   their own typed wrappers).
2. The value set is small, stable, and semantically closed enough
   that `#[non_exhaustive]` + `#[serde(other)] Unknown` bears the
   evolution cost better than the open-string approach.

Neither condition holds today for:

- `SessionStart::source` (`crates/knotch-agent/src/input.rs:53`) —
  matcher values `startup / resume / clear / compact / …`, zero
  knotch-side branches.
- `SessionEnd::reason` — surfaced at
  `crates/knotch-cli/src/cmd/hook/finalize_session.rs:9` as an
  opaque string passed to the agent helper; no kernel dispatch on
  the value.
- `PreToolUse::tool_name` — compared against `"Bash"` at
  `crates/knotch-agent/src/input.rs:139`; a single literal does
  not justify a closed `ToolName` enum.
- `SubagentStop::agent_type` — informational only.

All four remain `Option<CompactString>` (or `CompactString`).

## Active structural guarantee

`HookEvent` is `#[non_exhaustive]`
(`crates/knotch-agent/src/input.rs:52`) so Claude Code adding a
**new hook event** — `PostCompact`, `Notification`, future
variants — is **additive**: minor bump, no downstream breakage.
Open-string fields on existing variants absorb Claude Code's
evolution of **existing hooks** without any knotch change at all.

## Why

- Claude Code can rename / restructure source values without
  forcing knotch minor bumps.
- Alternative harnesses map to knotch without inheriting Claude-
  Code-shaped enum variants.
- Adopters that want branch-safety build a thin wrapper on their
  side — their choice, their maintenance surface.
- Constitution §V (hexagonal ports-and-adapters) is preserved:
  knotch-kernel types stay adapter-agnostic, adapter-specific
  values ride as metadata on the envelope.

## When to promote a string to a typed enum

Demonstrate both:

- A knotch-owned projection, kernel precondition, or lint rule
  that matches exhaustively on the value (not adopter code).
- The value set is small and stable enough that
  `#[non_exhaustive]` + `#[serde(other)] Unknown` bears the
  evolution cost better than the string approach.

Both must hold. One is not enough. Demonstrate via a failing test
(or a checked-in proof-of-concept that compiles) before the PR
lands, not in the PR description prose.

## Do not

- Add `SessionStartSource`, `SessionEndReason`, `ToolName`,
  `AgentType` enums without meeting **both** "when to promote"
  conditions above.
- Lock a variant set to a specific Claude Code release.
- Route compaction-specific business logic inside knotch by
  pattern-matching `SessionStart { source: "compact" }` —
  adopter-side skills own that decision.
- Propose a new `HookEvent` variant for a harness event that is
  not yet observed in production JSON from Claude Code. Wait for
  empirical evidence (a captured `hook_event_name` string); the
  `#[non_exhaustive]` marker ensures we can add it the moment it
  appears.
