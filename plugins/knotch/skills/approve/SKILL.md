# knotch-approve

Record a human approval — or rejection — against a prior event on
the active unit.

Shipped for **human-in-the-loop** workflows: an agent proposes a
gate decision / status transition / milestone ship; a named
reviewer signs off (or refuses) explicitly. The approval lands as
a first-class `ApprovalRecorded` event with a bounded rationale,
so projections, queries, and audits surface the signature rather
than it sitting in prose somewhere.

## Syntax

```bash
knotch approve <unit> <event-id> <decision> <rationale> --as <person>
```

Examples:

```bash
knotch approve signup-flow 019099fd-… approved \
  "Ship approved — infra-sec reviewed the new session token store" \
  --as alice@example.com

knotch approve signup-flow 019099fd-… rejected \
  "Revert requested — the revert lands in the next review cycle" \
  --as bob@example.com
```

## What fires

`EventBody::ApprovalRecorded { target, approver, decision, rationale }`.

`decision` reuses the same `approved / rejected / needs-revision /
deferred` vocabulary `knotch gate` records, so dashboards aggregate
both surfaces through one enum.

## What to check first

- `knotch log <unit>` — confirm the target event id exists (copy
  the UUID from the log output).
- Rationale length must clear the workflow's
  `min_rationale_chars()` floor (8 chars for the canonical `Knotch`
  workflow). A too-short rationale rejects with
  `PreconditionError::RationaleTooShort`.
- The same `approver` value may not record two approvals against
  the same target — knotch rejects with
  `PreconditionError::ApprovalAlreadyRecorded`. A different
  approver can always record their own independent approval.

## When NOT to use

- Gate decisions owned by the agent — use `/knotch-gate` instead.
  Approvals are for **human** endorsement of a prior event; gate
  entries are the agent's own decisions.
- Reverting / retracting a prior approval — use
  `knotch supersede <event-id>` to mark it no-longer-effective,
  then record a fresh approval if needed.
- Recording that an agent reviewed an artifact — use
  `/knotch-mark completed review` instead; `ApprovalRecorded`
  is specifically for a named human reviewer signing off,
  and is meaningless for agent-only flows.
