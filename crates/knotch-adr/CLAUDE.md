# knotch-adr

Specialised workflow for Architectural Decision Record (ADR)
lifecycles. Each ADR is one unit; the workflow carries it through
`proposed → active → {superseded, deprecated}`.

## Why a dedicated crate

The canonical `knotch-workflow::Knotch` workflow models dev-time
phases (specify / plan / build / review / ship). ADRs don't fit —
they have no such phase arc, they live and die as documents with
status transitions only. The `Adr` workflow explicitly models
the ADR shape without forcing the dev-time abstractions.

## Surface

| Type / Fn | Role |
|---|---|
| `Adr` | `WorkflowKind` marker for ADR lifecycles |
| `AdrPhase::Decided` | Single phase — marks "decision captured in writing" |
| `AdrId(CompactString)` | Free-form ADR slug; adopters pick the numbering scheme |
| `AdrGate::Unused` | Present only to satisfy `WorkflowKind::Gate`; never recorded |
| `frontmatter_schema()` | Builds a `FrontmatterSchema` requiring `id` / `title` / `status` / `created` |
| `lifecycle_fsm()` | Builds a `LifecycleFsm` with `superseded` + `deprecated` as terminal |
| `build_repository(root)` | File-backed `Adr` repository |
| `TEMPLATE` | `&'static str` Markdown skeleton with `{slug}` / `{title}` / `{today}` placeholders |

## Terminal statuses

`superseded` and `deprecated`:

- **superseded** — a newer ADR replaces this one. The newer ADR's
  frontmatter should carry `supersedes: <slug>` and this ADR's
  frontmatter `superseded_by: <newer-slug>`.
- **deprecated** — the decision no longer applies, but no
  successor exists yet.

## Adopter conventions we do not enforce

- **Numbering scheme** (`NNNN-slug` vs `YYYY-MM-DD-slug` vs
  free-form) — adopters choose when they construct an `AdrId`.
- **Section structure** (Status / Context / Decision / Consequences
  is the common shape, but adopters can add Alternatives
  Considered, Promotion Pipeline tiers, etc.) — the schema only
  validates frontmatter.
- **`supersedes` / `superseded_by` cross-links** — schema lets
  these through as arbitrary strings; adopters validate the
  linkage themselves if they want.
- **CLI surface** — the `TEMPLATE` constant ships; the skill /
  subcommand that stamps it out with today's date and the chosen
  slug lives in the adopter's code so `knotch-adr` doesn't drag
  in `clap`.

## Do not

- Emit `GateRecorded` against an ADR unit — the gate variant
  exists only to satisfy the trait surface.
- Reimplement the frontmatter schema — compose with
  `FrontmatterSchema::field` instead (see `knotch-schema`).
- Ship a second ADR template in this crate — if adopters want
  a different shape, they override `TEMPLATE` in their own
  scaffolding code.
