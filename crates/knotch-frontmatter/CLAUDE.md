# knotch-frontmatter

Optional utility for keeping a Markdown file's YAML frontmatter in
sync with a knotch unit's ledger status.

The log is authoritative (constitution §I); frontmatter is a
projection into a human-readable header block. Adopters that keep
one Markdown file per unit wire `sync_status_on_file` into their
status-transition hook.

## Surface

| Type | Role |
|---|---|
| `Document` | Parsed Markdown — split header + body. `get` / `set` / `remove` / `header` / `body` / `to_markdown`. |
| `sync_status_on_file(path, new_status)` | Atomic read-modify-write helper. No-op when the header already carries the given status. |
| `atomic_write(path, bytes)` | Low-level temp-file + rename. Reusable when adopters hand-construct a `Document`. |
| `FrontmatterError` | `Io { source }` / `NoFrontmatter` / `Yaml { source }` / `NotAnObject` / `Schema { source }`. |
| Re-exports from `knotch-schema` | `FrontmatterSchema`, `FieldSchema`, `FieldType`, `SchemaError` — so adopters don't need a second dep. |

## Do not

- Make this crate drive log events — observers propose events,
  this crate writes files. The adopter calls both from their
  hook in the correct order (log first, file second).
- Introduce a second YAML backend — we ship `yaml_serde` only.
  Adding another means dual-parse drift.
- Preserve the original YAML formatting byte-for-byte — we
  re-emit via `yaml_serde::to_string`, so comments and exotic
  ordering are lost. This is a deliberate trade-off against
  hand-crafted byte-level retention.

## When to reach for this crate

- Adopter workflows that keep one Markdown file (e.g. `spec.md`)
  per unit with a YAML frontmatter block and want the header's
  `status` field to follow every ledger `StatusTransitioned`.
- Projects that want `knotch show --format brief` and the
  on-disk spec header to agree on `status` after a transition.

If the adopter's unit doesn't have a Markdown file on disk, skip
this crate entirely.
