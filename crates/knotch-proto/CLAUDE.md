# knotch-proto

Wire format + schema versioning. **Zero I/O.** Isolates the on-disk
representation (RFC 8785 JCS canonicalization, log file header,
schema version, migration registry) so storage adapters and
fingerprint-verification tooling depend on it alone, without pulling
the full engine (`knotch-kernel`) and its observer / projection
machinery.

## Module map

| Module | Owns |
|---|---|
| `canonical` | `canonicalize(value) -> Vec<u8>` — RFC 8785 JCS via `serde_jcs`. The single approved canonicalization entry for fingerprinting. |
| `header` | `Header { kind: "__header__", schema_version, workflow, fingerprint_salt }` — the first line of every JSONL log file |
| `migration` | `SchemaMigrator` trait + `Registry` — no pre-registered migrators; wired in for the first breaking `SCHEMA_VERSION` bump |
| `lib` | `pub const SCHEMA_VERSION: u32` — bumped only on breaking wire changes |

## Version bump protocol

1. Change on-disk representation incompatibly.
2. Bump `SCHEMA_VERSION` in `lib.rs`.
3. Register a `SchemaMigrator` that rewrites old-form JSON to new-form.
4. Regenerate `docs/public_api/knotch-proto.baseline`.

## Do not

- Add crates that perform I/O (see
  @../../.claude/rules/no-unsafe.md and
  @../../.claude/rules/constitution.md §IV).
- Change the JCS dependency — fingerprint determinism depends on
  the exact byte output.
