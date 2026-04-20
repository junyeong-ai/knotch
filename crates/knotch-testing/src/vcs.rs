//! In-memory `Vcs` adapter for deterministic tests.

use std::{collections::HashMap, sync::Arc};

use arc_swap::ArcSwap;
use compact_str::CompactString;
use jiff::Timestamp;
use knotch_kernel::event::{CommitKind, CommitRef};
use knotch_vcs::{
    CommitFilter, Vcs,
    commit::{Commit, CommitStatus, Watermark},
    error::VcsError,
};

/// Pre-built commit fixture — what `InMemoryVcs` serves for a SHA.
#[derive(Debug, Clone)]
pub struct VcsFixture {
    /// Commit metadata.
    pub commit: Commit,
    /// Whether `verify_commit` should return `Verified` or `Pending`.
    pub status: CommitStatus,
    /// Parsed conventional-commits kind (for `CommitFilter::kinds`).
    pub kind: Option<CommitKind>,
}

impl VcsFixture {
    /// Construct a `Verified` fixture from minimal fields.
    #[must_use]
    pub fn verified(
        sha: impl Into<CompactString>,
        subject: impl Into<CompactString>,
        committed_at: Timestamp,
    ) -> Self {
        let sha = CommitRef::new(sha);
        Self {
            commit: Commit {
                sha,
                committed_at,
                subject: subject.into(),
                body: CompactString::default(),
                parents: Vec::new(),
            },
            status: CommitStatus::Verified,
            kind: None,
        }
    }

    /// Attach a body to the fixture.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<CompactString>) -> Self {
        self.commit.body = body.into();
        self
    }

    /// Attach a parsed commit kind (enables `CommitFilter::kinds`).
    #[must_use]
    pub fn with_kind(mut self, kind: CommitKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Mark the fixture as `Pending`.
    #[must_use]
    pub fn pending(mut self) -> Self {
        self.status = CommitStatus::Pending;
        self
    }
}

/// In-memory `Vcs` adapter. HEAD and the commit map are mutable via
/// `set_head` / `push_commit` so tests can script a sequence of
/// observations.
#[derive(Debug, Default, Clone)]
pub struct InMemoryVcs {
    state: Arc<State>,
}

#[derive(Debug, Default)]
struct State {
    head: ArcSwap<Option<CommitRef>>,
    commits: ArcSwap<Vec<VcsFixture>>,
    by_sha: ArcSwap<HashMap<String, VcsFixture>>,
}

impl InMemoryVcs {
    /// Construct an empty adapter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current HEAD.
    pub fn set_head(&self, head: CommitRef) {
        self.state.head.store(Arc::new(Some(head)));
    }

    /// Append a commit and update the ordered log. The most recent
    /// push becomes the first entry returned by `log_since`.
    pub fn push_commit(&self, fixture: VcsFixture) {
        let mut commits = (**self.state.commits.load()).clone();
        commits.insert(0, fixture.clone());
        self.state.commits.store(Arc::new(commits));

        let mut by_sha = (**self.state.by_sha.load()).clone();
        by_sha.insert(fixture.commit.sha.as_str().to_owned(), fixture);
        self.state.by_sha.store(Arc::new(by_sha));
    }

    /// Number of commits currently in the fixture.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.commits.load().len()
    }

    /// Is the fixture empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Vcs for InMemoryVcs {
    async fn verify_commit(&self, sha: &CommitRef) -> Result<CommitStatus, VcsError> {
        let key = sha.as_str();
        let status = self
            .state
            .by_sha
            .load()
            .get(key)
            .map(|f| f.status)
            .unwrap_or(CommitStatus::Missing);
        Ok(status)
    }

    async fn log_since(
        &self,
        since: Option<&CommitRef>,
        filter: &CommitFilter,
    ) -> Result<Vec<Commit>, VcsError> {
        let commits = self.state.commits.load();
        let mut out = Vec::new();
        for fixture in commits.iter() {
            if let Some(since) = since {
                if fixture.commit.sha == *since {
                    break;
                }
            }
            if !filter.kinds.is_empty() {
                match &fixture.kind {
                    Some(k) if filter.kinds.contains(k) => {}
                    _ => continue,
                }
            }
            out.push(fixture.commit.clone());
            if let Some(n) = filter.limit {
                if out.len() >= n {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn current_head(&self) -> Result<CommitRef, VcsError> {
        self.state
            .head
            .load()
            .as_ref()
            .clone()
            .ok_or_else(|| VcsError::HeadUnresolvable {
                source: Box::<dyn std::error::Error + Send + Sync + 'static>::from(
                    "in-memory VCS has no HEAD",
                ),
            })
    }

    async fn log_watermark(&self) -> Result<Watermark, VcsError> {
        let head = self.current_head().await?;
        Ok(Watermark { head, taken_at: Timestamp::now() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verify_missing_returns_missing() {
        let vcs = InMemoryVcs::new();
        let status = vcs.verify_commit(&CommitRef::new("abc")).await.expect("verify");
        assert_eq!(status, CommitStatus::Missing);
    }

    #[tokio::test]
    async fn verify_pushed_returns_verified() {
        let vcs = InMemoryVcs::new();
        vcs.push_commit(VcsFixture::verified(
            "abc1234",
            "feat: add thing",
            Timestamp::from_second(1_700_000_000).expect("ts"),
        ));
        let status = vcs
            .verify_commit(&CommitRef::new("abc1234"))
            .await
            .expect("verify");
        assert_eq!(status, CommitStatus::Verified);
    }

    #[tokio::test]
    async fn log_since_walks_newest_first() {
        let vcs = InMemoryVcs::new();
        let t0 = Timestamp::from_second(1_700_000_000).expect("ts");
        vcs.push_commit(VcsFixture::verified("aaa", "feat: a", t0));
        vcs.push_commit(VcsFixture::verified("bbb", "fix: b", t0));
        vcs.push_commit(VcsFixture::verified("ccc", "docs: c", t0));

        let log = vcs.log_since(None, &CommitFilter::default()).await.expect("log");
        assert_eq!(
            log.iter().map(|c| c.sha.as_str().to_owned()).collect::<Vec<_>>(),
            vec!["ccc".to_owned(), "bbb".to_owned(), "aaa".to_owned()]
        );
    }

    #[tokio::test]
    async fn log_since_excludes_the_since_sha() {
        let vcs = InMemoryVcs::new();
        let t = Timestamp::from_second(1_700_000_000).expect("ts");
        vcs.push_commit(VcsFixture::verified("aaa", "feat: a", t));
        vcs.push_commit(VcsFixture::verified("bbb", "fix: b", t));
        vcs.push_commit(VcsFixture::verified("ccc", "docs: c", t));

        let log = vcs
            .log_since(Some(&CommitRef::new("aaa")), &CommitFilter::default())
            .await
            .expect("log");
        let shas: Vec<_> = log.iter().map(|c| c.sha.as_str().to_owned()).collect();
        assert_eq!(shas, vec!["ccc".to_owned(), "bbb".to_owned()]);
    }

    #[tokio::test]
    async fn log_since_respects_kind_filter() {
        let vcs = InMemoryVcs::new();
        let t = Timestamp::from_second(1_700_000_000).expect("ts");
        vcs.push_commit(
            VcsFixture::verified("aaa", "feat: a", t).with_kind(CommitKind::Feat),
        );
        vcs.push_commit(
            VcsFixture::verified("bbb", "docs: b", t).with_kind(CommitKind::Docs),
        );
        let log = vcs
            .log_since(
                None,
                &CommitFilter { kinds: vec![CommitKind::Feat], limit: None },
            )
            .await
            .expect("log");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].sha.as_str(), "aaa");
    }
}
