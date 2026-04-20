# Public API baselines

Each published crate has a frozen `<crate>.baseline` file here. CI
runs `cargo public-api --diff-against docs/public_api/<crate>.baseline`
on every PR; deltas that are not explicitly approved (via a PR that
also updates the baseline) fail the build.

Baselines are updated only by maintainers as part of a release PR.

## Regeneration

```bash
# Requires nightly for `cargo public-api`:
rustup toolchain install nightly

for crate in knotch-kernel knotch-proto knotch-derive knotch-storage \
             knotch-lock knotch-vcs knotch-workflow knotch-schema \
             knotch-observer knotch-reconciler knotch-query \
             knotch-tracing knotch-testing knotch-linter \
             knotch-agent knotch-frontmatter knotch-adr; do
  cargo +nightly public-api --manifest-path crates/$crate/Cargo.toml \
      --simplified > docs/public_api/$crate.baseline
done
```

Or run `cargo xtask public-api` to regenerate every baseline in
one shot — the canonical list lives in `xtask/src/main.rs`
(`PUBLISHABLE_CRATES`).

## Semver policy

- `v0.x` pre-release: breaking changes allowed in minor versions.
  The baseline is re-cut at every minor bump.
- `v1.0+`: any API removal or signature change is a major bump.
  `cargo-semver-checks` enforces this alongside the baseline diff.

## Why both?

- `cargo-public-api` catches **additions** to the surface (so
  reviewers see when something becomes public unintentionally).
- `cargo-semver-checks` catches **breaking changes** (so releases
  can't accidentally bump a patch that removes a variant).

Together they form the surface-stability gate described in
`docs/strategy/long-term-plan.md`.
