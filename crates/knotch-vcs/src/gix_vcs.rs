//! `gix`-backed `Vcs` implementation.
//!
//! `gix::Repository` is `!Send`, so we wrap it in
//! `gix::ThreadSafeRepository` and clone into per-call `Repository`
//! handles inside `tokio::task::spawn_blocking`. This keeps the
//! adapter cheap to share (`Arc<GixVcs>`) while staying inside the
//! synchronous gix API.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use jiff::Timestamp;
use knotch_kernel::event::CommitRef;
use tokio::task;

use crate::{
    CommitFilter, Vcs,
    commit::{Commit, CommitStatus, Watermark},
    error::VcsError,
    parse::parse_commit_message,
};

/// Pure-Rust `gix`-backed VCS adapter.
#[derive(Debug, Clone)]
pub struct GixVcs {
    repo: Arc<gix::ThreadSafeRepository>,
    path: PathBuf,
}

impl GixVcs {
    /// Open a repository rooted at `path`.
    ///
    /// # Errors
    /// Returns `VcsError::OpenRepository` when gix cannot discover or
    /// open the repository.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, VcsError> {
        let path = path.as_ref().to_owned();
        let repo = gix::ThreadSafeRepository::open(&path).map_err(|e| {
            VcsError::OpenRepository {
                path: path.clone(),
                source: Box::new(e),
            }
        })?;
        Ok(Self { repo: Arc::new(repo), path })
    }

    /// Path the repository was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with_repo<F, R>(&self, f: F) -> impl std::future::Future<Output = Result<R, VcsError>> + Send
    where
        F: FnOnce(&gix::Repository) -> Result<R, VcsError> + Send + 'static,
        R: Send + 'static,
    {
        let handle = self.repo.clone();
        async move {
            task::spawn_blocking(move || {
                let repo = handle.to_thread_local();
                f(&repo)
            })
            .await
            .map_err(|join_err| VcsError::Backend(Box::new(join_err)))?
        }
    }
}

impl Vcs for GixVcs {
    async fn verify_commit(&self, sha: &CommitRef) -> Result<CommitStatus, VcsError> {
        let wanted = sha.as_str().to_owned();
        self.with_repo(move |repo| {
            match repo.rev_parse_single(wanted.as_str()) {
                Ok(id) => match repo.find_commit(id) {
                    Ok(_) => Ok(CommitStatus::Verified),
                    Err(_) => Ok(CommitStatus::Missing),
                },
                Err(_) => Ok(CommitStatus::Missing),
            }
        })
        .await
    }

    async fn log_since(
        &self,
        since: Option<&CommitRef>,
        filter: &CommitFilter,
    ) -> Result<Vec<Commit>, VcsError> {
        let since_sha = since.map(|s| s.as_str().to_owned());
        let filter_kinds = filter.kinds.clone();
        let limit = filter.limit;
        self.with_repo(move |repo| {
            let head = repo.head_commit().map_err(|e| VcsError::HeadUnresolvable {
                source: Box::new(e),
            })?;
            let since_sha_norm = since_sha.as_ref().map(|s| s.to_ascii_lowercase());

            let walk = repo
                .rev_walk([head.id])
                .all()
                .map_err(|e| VcsError::Backend(Box::new(e)))?;
            let mut out = Vec::new();
            for info in walk {
                let info = info.map_err(|e| VcsError::Backend(Box::new(e)))?;
                let id_str = info.id.to_string();
                if let Some(since) = since_sha_norm.as_deref() {
                    if id_str.starts_with(since) {
                        break;
                    }
                }
                let commit = repo
                    .find_commit(info.id)
                    .map_err(|e| VcsError::Backend(Box::new(e)))?;
                let built = build_commit(&commit)?;
                if !filter_kinds.is_empty() {
                    if let Ok(parsed) = parse_commit_message(
                        built.sha.clone(),
                        &commit_message_string(&commit),
                    ) {
                        if !filter_kinds.contains(&parsed.kind) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                out.push(built);
                if let Some(n) = limit {
                    if out.len() >= n {
                        break;
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    async fn current_head(&self) -> Result<CommitRef, VcsError> {
        self.with_repo(move |repo| {
            let head = repo.head_commit().map_err(|e| VcsError::HeadUnresolvable {
                source: Box::new(e),
            })?;
            Ok(CommitRef::new(head.id.to_string()))
        })
        .await
    }

    async fn log_watermark(&self) -> Result<Watermark, VcsError> {
        let head = self.current_head().await?;
        Ok(Watermark { head, taken_at: Timestamp::now() })
    }
}

fn build_commit(commit: &gix::Commit<'_>) -> Result<Commit, VcsError> {
    let message = commit_message_string(commit);
    let (subject, body) = split_subject_body(&message);
    let when = commit
        .committer()
        .map_err(|e| VcsError::Backend(Box::new(e)))?
        .time
        .seconds;
    let committed_at = Timestamp::from_second(when)
        .unwrap_or_else(|_| Timestamp::from_second(0).expect("epoch is valid"));
    let parents = commit
        .parent_ids()
        .map(|id| CommitRef::new(id.to_string()))
        .collect::<Vec<_>>();
    Ok(Commit {
        sha: CommitRef::new(commit.id.to_string()),
        committed_at,
        subject: subject.into(),
        body: body.into(),
        parents,
    })
}

fn commit_message_string(commit: &gix::Commit<'_>) -> String {
    commit
        .message_raw_sloppy()
        .to_string()
}

fn split_subject_body(message: &str) -> (String, String) {
    let mut lines = message.splitn(2, '\n');
    let subject = lines.next().unwrap_or("").trim().to_owned();
    let rest = lines.next().unwrap_or("");
    let body = rest.trim_start_matches('\n').to_owned();
    (subject, body)
}
