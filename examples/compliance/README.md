# knotch-example-compliance

Audit-heavy workflow where every event carries **typed extension metadata** (reviewer id, timestamp). Demonstrates the non-trivial `Extension = T` path.

- **Phases** — `Submitted` → `Reviewed` → `Approved` → `Implemented` → `Audited`
- **Milestone** — `ChangeId`
- **Gates** — `SecurityReview`, `ComplianceReview`, `ApprovalBoard`
- **Extension** — `AuditMeta { reviewer, stamp }`
- **Terminal statuses** — `approved_closed`, `rejected_closed`, `abandoned`
- **Rationale floor** — 32 chars

## Run

```bash
cargo run -p knotch-example-compliance
```

## Why typed extension pays off

- Every event automatically carries the reviewer/time stamp — no sidecar table, no join query.
- Compliance queries walk `log.events().iter().filter_map(|e| &e.extension.reviewer)` — pure projection.
- `ExtensionKind::check_extension` lets you reject events that lack required fields at the Repository boundary. (This example keeps the check trivial; production implementations enforce "reviewer must be set on terminal-status transitions".)

## When NOT to use extension metadata

- Ephemeral UI state — that belongs in a projection cache, not on the event.
- Large blobs (≥ 8 KiB) — extension payloads are serialized JCS and hashed into the fingerprint on every append.
- Data that would make JCS re-serialization unstable (unordered maps of dynamic keys).
