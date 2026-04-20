#!/usr/bin/env node
// Reference Node.js observer for the knotch SubprocessObserver wire
// protocol. Mirrors `examples/subprocess-observer-py/observer.py`
// verbatim in behaviour — same request/response shape, same exit
// code contract — so the only difference between the two is the
// host language.
//
// Reads a single JSON-line request on stdin, emits a single JSON-
// line response on stdout. Exit 0 on success; exit 1 on transient
// failure; exit 2 on permanent failure. Anything else is classified
// as a crash and the reconciler captures stderr for diagnostics.
//
// ## Request shape
//
// ```json
// {
//   "unit": "feature-x",
//   "head": "abc1234",
//   "taken_at": "2026-04-19T10:00:00Z",
//   "events": [ /* Event<W> objects, filtered by subscribes */ ],
//   "budget": { "max_proposals": 128 }
// }
// ```
//
// ## Response shape
//
// ```json
// { "proposals": [ /* Proposal<W> objects */ ] }
// ```
//
// Empty `proposals` is fine — the observer had nothing to say.
//
// ## Registering
//
// Declare in `knotch.toml`:
//
// ```toml
// [[observers]]
// name = "my-observer"
// binary = "examples/subprocess-observer-node/observer.mjs"
// subscribes = ["phase_completed", "milestone_shipped"]
// deterministic = true
// timeout_ms = 10000
// ```

"use strict";

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function main() {
  let request;
  try {
    const raw = await readStdin();
    request = JSON.parse(raw);
  } catch (err) {
    process.stderr.write(`stdin is not valid JSON: ${err.message}\n`);
    process.exit(2);
  }

  const unit = request.unit;
  const events = request.events ?? [];
  const budget = request.budget?.max_proposals ?? 128;

  // --- Your observer logic goes here. ---
  // Inspect `events`, correlate with external state (git, files,
  // HTTP, database, ...), and emit `Proposal<W>` objects.
  const proposals = [];

  // Example: for every PhaseCompleted on `unit`, emit nothing —
  // this template observes but doesn't propose.
  //
  // Replace with real logic for your adopter (e.g. re-scan spec
  // frontmatter, re-run design-lint, re-measure bundle size).
  for (const _event of events) {
    if (proposals.length >= budget) break;
    void unit; // reserved for real per-unit correlation
  }

  process.stdout.write(JSON.stringify({ proposals }));
  process.stdout.write("\n");
  process.exit(0);
}

main().catch((err) => {
  process.stderr.write(`unhandled error: ${err.stack ?? err.message}\n`);
  process.exit(2);
});
