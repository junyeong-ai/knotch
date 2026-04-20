# subprocess-observer-node

Reference Node.js observer for the knotch `SubprocessObserver`
wire protocol. Functionally identical to
[`../subprocess-observer-py/observer.py`](../subprocess-observer-py/observer.py)
— same request JSON, same response JSON, same exit-code contract —
so adopters on Node / TypeScript don't have to reverse-engineer
the Python example.

## Requirements

Node 18+ (for top-level `await` + readable async iteration on
`process.stdin`). No npm dependencies.

## Running

```bash
chmod +x examples/subprocess-observer-node/observer.mjs
echo '{"unit":"demo","events":[],"budget":{"max_proposals":8}}' \
  | examples/subprocess-observer-node/observer.mjs
# → {"proposals":[]}
```

## Registering in `knotch.toml`

```toml
[[observers]]
name = "my-observer"
binary = "examples/subprocess-observer-node/observer.mjs"
subscribes = ["phase_completed", "milestone_shipped"]
deterministic = true
timeout_ms = 10000
```

## Wire protocol

See the header comment in `observer.mjs` for the full request
and response shapes. Exit codes:

| Code | Meaning |
|---|---|
| `0` | Success — stdout carries `{ "proposals": [...] }` |
| `1` | Transient failure — reconciler retries under the same anchor |
| `2` | Permanent failure — reconciler emits `ReconcileFailed` and stops retrying this anchor |
| other | Crash — stderr captured for diagnostics |

## Adopter logic

Replace the template body in `observer.mjs` with the real
correlation step. Typical shapes:

- Re-read a markdown file's frontmatter and emit a
  `GateRecorded` when a `[NEEDS CLARIFICATION]` disappears.
- Hit an HTTP endpoint and emit a `MilestoneVerified` when a
  deploy comes green.
- Re-measure a cost metric (bundle size, p95 latency) and emit
  a `GateRecorded` with `decision: pass|fail`.

The protocol is intentionally thin so adopters can ship logic
in whatever Node runtime best matches their infrastructure
(Bun, Deno, Node, monorepo or one-off).
