//! VCS commit value types.

use compact_str::CompactString;
use jiff::Timestamp;
use knotch_kernel::event::{CommitKind, CommitRef};
// `CommitStatus` is owned by the kernel (it ships on
// `EventBody::MilestoneShipped`); re-exported for ergonomics so VCS
// callers don't cross into a second crate for one enum.
pub use knotch_kernel::event::CommitStatus;

/// A raw commit as returned by a `Vcs::log_since` walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// 40-char lowercase SHA.
    pub sha: CommitRef,
    /// Author-commit timestamp.
    pub committed_at: Timestamp,
    /// Single-line subject (first line of the commit message).
    pub subject: CompactString,
    /// Body following the blank-line separator, or empty.
    pub body: CompactString,
    /// Parent SHAs, first is the canonical parent.
    pub parents: Vec<CommitRef>,
}

/// Parsed commit header + body hints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommit {
    /// Commit identity.
    pub sha: CommitRef,
    /// Conventional-commits kind (plus synthetic `Revert`).
    pub kind: CommitKind,
    /// Optional scope from `<kind>(<scope>): ...`.
    pub scope: Option<CompactString>,
    /// Breaking-change marker (`!` after kind/scope or a body
    /// `BREAKING CHANGE:` footer).
    pub breaking: bool,
    /// Single-line subject.
    pub subject: CompactString,
    /// Full body text.
    pub body: CompactString,
    /// Populated when the body contains `This reverts commit <sha>.`.
    pub reverts: Option<CommitRef>,
}

/// Link between a revert and its original commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertLink {
    /// The commit being reverted.
    pub original: CommitRef,
    /// The revert commit itself.
    pub revert: CommitRef,
}

/// Observer watermark — typically the HEAD at snapshot time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    /// HEAD SHA at snapshot time.
    pub head: CommitRef,
    /// When the snapshot was taken.
    pub taken_at: Timestamp,
}
