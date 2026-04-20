#!/usr/bin/env python3
"""Reference Python observer for the knotch SubprocessObserver wire
protocol.

Reads a single JSON-line request on stdin, emits a single JSON-line
response on stdout. Exit 0 on success; exit 1 on transient failure;
exit 2 on permanent failure. Anything else is classified as a crash
and the reconciler captures stderr for diagnostics.

## Request shape

```json
{
  "unit": "feature-x",
  "head": "abc1234",
  "taken_at": "2026-04-19T10:00:00Z",
  "events": [ /* Event<W> objects, filtered by subscribes */ ],
  "budget": { "max_proposals": 128 }
}
```

## Response shape

```json
{ "proposals": [ /* Proposal<W> objects */ ] }
```

Empty `proposals` is fine — the observer had nothing to say.

## Registering

Declare in `knotch.toml`:

```toml
[[observers]]
name = "my-observer"
binary = "examples/subprocess-observer-py/observer.py"
subscribes = ["phase_completed", "milestone_shipped"]
deterministic = true
timeout_ms = 10000
```
"""

from __future__ import annotations

import json
import sys
from typing import Any


def main() -> int:
    try:
        request = json.loads(sys.stdin.read())
    except json.JSONDecodeError as exc:
        print(f"stdin is not valid JSON: {exc}", file=sys.stderr)
        return 2

    unit: str = request["unit"]
    events: list[dict[str, Any]] = request.get("events", [])
    budget: int = request.get("budget", {}).get("max_proposals", 128)

    # --- Your observer logic goes here. ---
    # Inspect `events`, correlate with external state (git, files,
    # HTTP, database, ...), and emit `Proposal<W>` objects.
    proposals: list[dict[str, Any]] = []

    # Example: for every PhaseCompleted on `unit`, emit nothing —
    # this template observes but doesn't propose.
    #
    # Replace with real logic for your adopter (e.g. re-scan
    # spec frontmatter, re-run design-lint, re-measure bundle size).
    for _event in events:
        if len(proposals) >= budget:
            break

    json.dump({"proposals": proposals}, sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
