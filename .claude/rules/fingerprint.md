---
paths:
  - "crates/knotch-kernel/src/fingerprint.rs"
  - "crates/knotch-storage/src/file_repository.rs"
  - "crates/knotch-testing/src/repo.rs"
  - "crates/knotch-proto/**"
---

# Fingerprint is closed

Two proposals share a `Fingerprint` when they describe the same
workflow mutation. Replaying a proposal with an existing fingerprint
is a silent no-op (dedup). **Users cannot swap the algorithm.**

## Dedup tuple

```
{
  "workflow":   W::NAME,
  "body":       serde_json::to_value(body),
  "supersedes": Option<EventId>,
}
```

Serialized via **RFC 8785 JCS** (`serde_jcs::to_vec`) — byte-identical
across platforms, serde versions, and key-insertion orders.

Final hash: `blake3(W::fingerprint_salt() || canonical_bytes)`. Salt
default is `W::NAME` — so two workflows sharing a storage root
cannot collide.

Canonical: `crates/knotch-kernel/src/fingerprint.rs::fingerprint_proposal`,
`::fingerprint_event`. Both go through the private
`fingerprint_parts` helper — do not reimplement.

## Why closed

A user-swappable `Fingerprinter` would silently invalidate stored
logs when the swap happens (stored events would re-hash to new
fingerprints, causing spurious duplicates or missed dedup).

Workflows that need different dedup semantics change
`W::fingerprint_salt` (namespace isolation) or define a new
`WorkflowKind`.

## Salt evolution rule

A change to `W::fingerprint_salt()` silently invalidates every
stored log that was written with the prior salt — dedup against
those events would produce false negatives. To keep this safe, the
`FileRepository` persists the base64-encoded salt in each log's
header line and refuses to load or append if the stored salt does
not match `W::fingerprint_salt()` on the current build
(`RepositoryError::SaltMismatch`, see
`crates/knotch-storage/src/file_repository.rs::check_header_salt`).

Changing the salt is therefore a **breaking wire change**: bump
`W::SCHEMA_VERSION` and ship a `SchemaMigrator` that re-fingerprints
stored events against the new salt.

## Agent-facing consequence

When an agent re-submits the same proposal for retry safety, the
rejection is *not* an error — it means the prior append landed. Treat
`AppendReport::rejected[].reason == "duplicate"` as the success
signal for at-least-once delivery.

## Parity guarantee

`InMemoryRepository` and `FileRepository` both use the same
`fingerprint_proposal` — enforced by the parity test
`crates/knotch-storage/tests/fingerprint_parity.rs`.
