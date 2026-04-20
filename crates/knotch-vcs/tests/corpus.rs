//! Corpus-diff integration test.
//!
//! Builds a known git repository with a scripted commit history, then
//! reads it back through `GixVcs` and asserts that three consecutive
//! runs produce byte-identical snapshots. Exit criterion for Phase 3:
//! "fixture repo yields identical output on 3 runs."

use std::{path::Path, process::Command};

use knotch_kernel::event::CommitRef;
use knotch_vcs::{CommitFilter, GixVcs, Vcs, parse::parse_commit_message};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Knotch Test")
        .env("GIT_AUTHOR_EMAIL", "test@knotch.dev")
        .env("GIT_COMMITTER_NAME", "Knotch Test")
        .env("GIT_COMMITTER_EMAIL", "test@knotch.dev")
        .env("GIT_AUTHOR_DATE", "2026-04-18T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-04-18T12:00:00Z")
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn build_fixture_repo(root: &Path) {
    std::fs::create_dir_all(root).expect("mkdir");
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    for (idx, (name, msg)) in [
        ("a.txt", "feat(core): initial surface\n\nFirst commit."),
        ("b.txt", "fix(core): correct boundary check"),
        ("c.txt", "docs: document knotch principles"),
        ("d.txt", "refactor!: drop deprecated helper\n\nBREAKING CHANGE: removed."),
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(name);
        std::fs::write(&path, format!("content {idx}\n")).expect("write");
        git(root, &["add", name]);
        git(root, &["commit", "--quiet", "-m", msg]);
    }
}

fn snapshot(vcs: &GixVcs) -> Vec<String> {
    tokio::runtime::Runtime::new().expect("rt").block_on(async {
        let log = vcs.log_since(None, &CommitFilter::default()).await.expect("log_since");
        log.into_iter().map(|c| format!("{} | {}", c.sha, c.subject)).collect()
    })
}

#[test]
fn three_runs_produce_identical_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    build_fixture_repo(dir.path());

    let vcs = GixVcs::open(dir.path()).expect("open");
    let first = snapshot(&vcs);
    let second = snapshot(&vcs);
    let third = snapshot(&vcs);

    assert_eq!(first, second, "second run diverged from first");
    assert_eq!(second, third, "third run diverged from second");
    assert_eq!(first.len(), 4, "expected 4 commits in fixture");
}

#[tokio::test]
async fn current_head_and_verify_agree() {
    let dir = tempfile::tempdir().expect("tempdir");
    build_fixture_repo(dir.path());
    let vcs = GixVcs::open(dir.path()).expect("open");
    let head = vcs.current_head().await.expect("head");
    let status = vcs.verify_commit(&head).await.expect("verify");
    assert_eq!(status, knotch_vcs::CommitStatus::Verified);
}

#[tokio::test]
async fn unknown_commit_returns_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    build_fixture_repo(dir.path());
    let vcs = GixVcs::open(dir.path()).expect("open");
    let status = vcs
        .verify_commit(&CommitRef::new("0000000000000000000000000000000000000000"))
        .await
        .expect("verify");
    assert_eq!(status, knotch_vcs::CommitStatus::Missing);
}

#[tokio::test]
async fn log_since_skips_prior_commits() {
    let dir = tempfile::tempdir().expect("tempdir");
    build_fixture_repo(dir.path());
    let vcs = GixVcs::open(dir.path()).expect("open");
    let full = vcs.log_since(None, &CommitFilter::default()).await.expect("full");
    assert_eq!(full.len(), 4);

    let cutoff = full[2].sha.clone();
    let partial = vcs.log_since(Some(&cutoff), &CommitFilter::default()).await.expect("partial");
    assert_eq!(partial.len(), 2, "expected 2 newer commits than cutoff");
}

#[tokio::test]
async fn parser_recognizes_fixture_kinds() {
    let dir = tempfile::tempdir().expect("tempdir");
    build_fixture_repo(dir.path());
    let vcs = GixVcs::open(dir.path()).expect("open");
    let log = vcs.log_since(None, &CommitFilter::default()).await.expect("log");

    let mut breaking_seen = false;
    let mut scope_seen = false;
    for commit in log {
        let parsed = parse_commit_message(
            commit.sha.clone(),
            &format!("{}\n\n{}", commit.subject, commit.body),
        )
        .expect("parse");
        if parsed.breaking {
            breaking_seen = true;
        }
        if parsed.scope.as_deref() == Some("core") {
            scope_seen = true;
        }
    }
    assert!(breaking_seen, "expected a breaking commit in the fixture");
    assert!(scope_seen, "expected a scoped commit in the fixture");
}
