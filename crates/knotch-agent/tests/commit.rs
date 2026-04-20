//! Smoke tests for `knotch_agent::commit` against the
//! `InMemoryRepository` (parity with the file-backed adapter is
//! enforced in `knotch-storage/tests/fingerprint_parity.rs`).

use knotch_agent::{HookOutput, commit};
use knotch_kernel::UnitId;
use knotch_testing::InMemoryRepository;
use knotch_workflow::Knotch;

#[tokio::test]
async fn check_passes_through_non_conventional_messages() {
    let repo = InMemoryRepository::<Knotch>::new(Knotch);
    let unit = UnitId::new("signup-flow");
    let out = commit::check::<Knotch, InMemoryRepository<Knotch>>(&repo, &unit, "wip")
        .await
        .unwrap();
    assert_eq!(out, HookOutput::Continue);
}

#[tokio::test]
async fn check_allows_fresh_milestone() {
    let repo = InMemoryRepository::<Knotch>::new(Knotch);
    let unit = UnitId::new("signup-flow");
    let out = commit::check::<Knotch, InMemoryRepository<Knotch>>(&repo, &unit, "feat: add sso login")
        .await
        .unwrap();
    assert_eq!(out, HookOutput::Continue);
}

#[tokio::test]
async fn check_ignores_scope_prefix() {
    let repo = InMemoryRepository::<Knotch>::new(Knotch);
    let unit = UnitId::new("signup-flow");
    let out = commit::check::<Knotch, InMemoryRepository<Knotch>>(&repo, &unit, "feat(auth): add sso")
        .await
        .unwrap();
    assert_eq!(out, HookOutput::Continue);
}
