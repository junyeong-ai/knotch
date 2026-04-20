# knotch-testing

In-memory fakes + simulation harness. **Dev-dependency only** —
never pull this crate into a production binary. Production code
that needs an in-memory path should ask why (usually it doesn't).

@../../.claude/rules/no-unsafe.md
@../../.claude/rules/testing.md

## Surface

| Module | Owns |
|---|---|
| `repo::InMemoryRepository<W>` | `Repository<W>` impl backed by `tokio::sync::RwLock`-guarded `HashMap`s. Matches `FileRepository`'s contract byte-for-byte so fingerprint parity, precondition dispatch, and cache semantics transfer over. |
| `vcs::{InMemoryVcs, VcsFixture}` | `Vcs` impl used by observer tests. Fixture builder stamps commit history without shelling out to `git`. |
| `sim` | Simulation harness — scripted event sequences for reconciler / observer / concurrency tests. |

## Extension recipe — add a new in-memory adapter

1. Mirror the port trait's contract exactly — the whole point is
   parity. Copy the test table from `FileRepository` and
   `GixVcs`; paste the relevant rows under `crates/<port>/tests/`.
2. Never diverge from the file-backed semantics. If you find
   yourself adding a test that only passes in-memory, you've
   introduced a gap — fix the in-memory impl instead.
3. Add a parity test under the appropriate port crate
   (`crates/knotch-storage/tests/fingerprint_parity.rs` is the
   template) so the two implementations stay aligned as both
   evolve.

## Do not

- Ship this crate as a runtime dep — the `[dev-dependencies]`
  gate is load-bearing. A runtime use would mean "we have
  production code that accepts unsynced in-memory state", which
  violates constitution §I.
- Add adopter-specific fixtures — a test fixture for
  webloom-shaped units lives in webloom's test tree, not here.
- Reimplement fingerprint / precondition logic — the whole point
  of parity is that kernel dispatch runs unchanged.
