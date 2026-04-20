---
paths:
  - "crates/**/src/**/*.rs"
---

# `#![forbid(unsafe_code)]` is universal

No crate may contain `unsafe` blocks. No exceptions, including the
file-lock and syscall layers.

## Why enforceable without cost

Every low-level concern has a safe wrapper in 2026:

| Concern | Safe crate |
|---|---|
| Syscalls (fs, process, net) | `rustix` |
| Advisory file locks | `fs4` (rustix-backed) |
| Git plumbing | `gix` (pure Rust) |

## Verification

- Workspace lint: `[workspace.lints.rust]` has `unsafe_code = "forbid"`.
- Direct check: `grep -R "unsafe " crates/*/src/` — expected empty.
- CI: `cargo clippy --workspace --all-targets -- -D warnings` catches
  any `unsafe` that slips through.

## Why this matters for agents

The "agent writes a fix, CI verifies" loop only holds when the
verification surface is solid. `unsafe` punctures it — behavior
becomes undefined and test outcomes stop being evidence. The zero-
unsafe rule keeps every agent proposal machine-checkable.

## Where agents are tempted

Locking and atomic rename look like they need `unsafe`. They do not:

- `FileLock` uses `fs4::tokio::AsyncFileExt` + `rustix::process::test_kill_process`
  (`crates/knotch-lock/src/file_lock.rs`).
- Atomic write uses `tokio::fs::rename` on POSIX and `MoveFileEx`
  retry on Windows, both via safe wrappers
  (`crates/knotch-storage/src/atomic.rs`).

If a proposed change reaches for `unsafe`, it is wrong. Re-derive
the design from a safe primitive.
