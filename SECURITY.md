# Security Policy

## Supported versions

| Version | Status |
|---|---|
| pre-`v1.0` | latest minor only |
| `v1.x` (when released) | 24-month LTS from the first `v1.0` |

## Reporting a vulnerability

Use GitHub's **private vulnerability reporting** on the `junyeong-ai/knotch`
repository, or email security@knotch.dev.

We operate a **90-day coordinated disclosure** window. Reporters are
credited in the advisory unless they request otherwise.

## Security invariants

- **`#![forbid(unsafe_code)]`** in every crate. Safe FFI via `rustix`
  + `fs4` + `gix`.
- **No network I/O in default features.** Every networked capability
  is opt-in.
- **Sensitive fields** (operator name, agent id, person) are marked
  with `#[derive(Sensitive)]`; tracing subscribers hash them by
  default.
- **Supply chain**: `cargo-deny`, `cargo-audit`, `cargo-vet` gate
  every PR; SBOM via `cargo-cyclonedx` published with each release.
