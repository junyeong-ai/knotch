# knotch-tracing

Stable tracing attribute schema + span helpers. The attribute
keys are part of the public surface — dashboards and alert rules
that pin to these names survive knotch releases because the
`cargo-public-api` diff catches any rename.

@../../.claude/rules/no-unsafe.md
@../../.claude/rules/causation.md

## Surface

| Module | Owns |
|---|---|
| `attrs` | `const KNOTCH_*` attribute-name constants. `knotch.unit`, `knotch.event.id`, `knotch.event.kind`, `knotch.causation.agent_id`, and so on. |
| `spans` | Span builders that pre-populate the attribute keys from a `Causation` / `Event<W>`. Integrates with `tracing::info_span!` so downstream subscribers receive pre-tagged spans. |

Downstream integrations (OpenTelemetry export, `metrics` counters,
Prometheus scrape) stay out-of-process — subscribers of the
`tracing` events we emit pick up the `knotch.*` attributes via
their own layers. This crate stays dependency-light so adopters
that only use `tracing` pay zero cost.

## Extension recipe — add a new attribute key

1. Add a `pub const KNOTCH_<key>: &str = "knotch.<key>"` in
   `attrs.rs`. Keep the namespace prefix — it's the grep handle
   ops teams use.
2. Populate it in every span constructor that logically carries
   it (usually the `span_for_event` family in `spans.rs`).
3. Regenerate the public-API baseline — a new attribute widens
   the stable surface and needs to show up in the diff.
4. Document the attribute's meaning + expected value type in
   the constant's doc comment; ops dashboards read these
   verbatim.

## Do not

- Rename an existing attribute without a `SCHEMA_VERSION`-style
  bump on the tracing crate itself — external dashboards break
  silently.
- Route sensitive fields (`Principal::Person`, `Principal::Agent`
  raw identifiers) through the un-hashed path — the subscriber
  must hash `Sensitive` markers before emission.
- Pull an OTel / metrics bridge into this crate directly —
  adopters wire those into their own subscriber stack; this
  crate stays a pure attribute-schema + span-helper surface.
