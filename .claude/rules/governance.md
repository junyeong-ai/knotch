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
Precedent: `knotch-frontmatter` (Markdown ↔ ledger status sync)
and `knotch-adr` (ADR lifecycle) both live as optional Tier-5
crates so an adopter who needs neither pays zero cost.
