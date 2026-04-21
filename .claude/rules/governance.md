---
paths:
  - "**"
---

# knotch scope contract

knotch is a **shared ledger for agent-driven workflows**, not a
workflow engine. Every PR that widens the public surface has to
answer the four questions below; every maintainer reviewing has
to refuse the features on the veto list.

## Four-step PR rubric

1. **Structural invariant.** Does this enforce an invariant of
   append-only workflow ledgers (monotonic time, dedup, lock
   serialisation, projection purity)? If no — stop. This belongs
   in a workflow crate, an adopter repo, or an optional Tier-5
   crate, not in the kernel or mandatory-tier surface.
2. **Universality.** Name two hypothetical adopters beyond the
   requester that would need this. If you can't, refuse and
   point to a workflow-side extension.
3. **Opt-in shape.** Can this be expressed as an optional crate
   (observer, validator, adapter, specialised workflow) rather
   than kernel surface? Opt-in mistakes cost one `cargo remove`;
   kernel mistakes cost 18 months. Always prefer opt-in.
4. **Public-API impact.** Does this change
   `docs/public_api/*.baseline`? If yes, regenerate in this PR
   and tag the semver implication (patch / minor / major).

## Explicit vetoes

Refuse without further discussion:

- **Team dashboards, roadmap planners, review aggregators.**
  Orchestration, not ledger. Universality ratio is project-
  specific (team taxonomy, review pipeline, merge-freeze rules).
  Ship a ≤30-LOC composition example at most.
- **Template / design / deploy-target catalogs.** Runtime, not
  ledger. Adopters own their own catalogs.
- **Business policy** (reviewer rank, approval chain, SLA
  enforcement, access control). Consuming project's
  `.claude/rules/`.
- **Scope vocabulary expansion** beyond the current variants.
  Workflows extend via their own Phase / Gate / Milestone types.
- **Project-branded rule files.** Adopter-specific rules stay
  in the adopter's own repository. knotch ships only
  ledger-structural rules.
- **ADR section vocabularies, numbering schemes, promotion
  pipelines.** `knotch-adr` ships the minimal frontmatter schema
  + lifecycle FSM and stops there — sections and numbering stay
  adopter sovereignty.
- **Re-introducing a multi-workflow "preset" concept.** The
  single canonical workflow (`knotch_workflow::Knotch`) plus
  case-study forks under `examples/workflow-*-case-study/` is
  deliberate — restoring the "which preset do I pick?" fork
  would re-introduce a decision adopters don't need to make.

## When a feature walks the line

Ship it as an opt-in crate (Tier 4 or 5), not in the kernel.
Recent precedent: `knotch-frontmatter` (Markdown ↔ ledger
status sync) and `knotch-adr` (ADR lifecycle) both live as
optional Tier-5 crates because fewer than half of hypothetical
adopters need them. A third adopter who doesn't use Markdown
files or ADRs pays zero cost.

## Metrics (evaluated quarterly)

- **M1 — kernel public-API growth.** ≤5% lines added per
  quarter after v1.0. A surging kernel signals workflow-specific
  leakage.
- **M2 — workflow divergence.** If a new adopter workflow forces
  a kernel change, the pattern is escaping policy. Audit via
  the rubric above before accepting.
- **M3 — adopter count.** Target: three independent workflows in
  production within twelve months of v1.0.
- **M4 — shared-rule reuse.** Adopters who `@`-import this
  rule tree vs. copy it. Higher import ratio = universality is
  holding.
- **M5 — feature-request refusal rate.** Healthy: 40-60%. Lower
  → scope contract drifting; higher → library too narrow to
  matter.
