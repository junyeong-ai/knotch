//! `GitLogObserver` — walks the VCS log since the cache watermark and
//! emits `MilestoneShipped` for conventional-commit implementation
//! kinds (feat / fix / refactor / perf), plus `MilestoneReverted`
//! for commits that cite `This reverts commit <sha>.`.
//!
//! Conventional Commits grammar is handled by
//! `knotch_vcs::parse::parse_commit_message`. The mapping from
//! commit to milestone is workflow-specific: the observer accepts a
//! `MilestoneResolver` closure that tells it "given this parsed
//! commit, which milestone does it ship?". Workflows whose
//! milestones are extracted from commit bodies supply their own
//! resolver; simple workflows can use
//! [`subject_prefix_resolver`] as a starter.

use std::{marker::PhantomData, sync::Arc};

use compact_str::CompactString;
use knotch_kernel::{
    Causation, Proposal, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{CommitKind, CommitRef, EventBody},
};
use knotch_vcs::{CommitFilter, Vcs, parse::parse_commit_message};

use crate::{Observer, ObserveContext, ObserverError};

/// Function that converts a parsed commit to a workflow milestone.
/// Returns `None` if the commit does not ship any milestone known
/// to the workflow.
pub type MilestoneResolver<W> = Arc<
    dyn Fn(&knotch_vcs::ParsedCommit) -> Option<<W as WorkflowKind>::Milestone>
        + Send
        + Sync
        + 'static,
>;

/// Git-log-walking observer.
pub struct GitLogObserver<V, W: WorkflowKind> {
    vcs: Arc<V>,
    resolver: MilestoneResolver<W>,
    _marker: PhantomData<fn() -> W>,
}

impl<V, W: WorkflowKind> GitLogObserver<V, W>
where
    V: Vcs,
{
    /// Construct a new observer. The unit id is read from
    /// `ObserveContext` at call time — the observer itself is
    /// workflow-wide, so one instance can serve many units.
    pub fn new(vcs: Arc<V>, resolver: MilestoneResolver<W>) -> Self {
        Self { vcs, resolver, _marker: PhantomData }
    }
}

impl<V, W> Observer<W> for GitLogObserver<V, W>
where
    V: Vcs,
    W: WorkflowKind,
{
    fn name(&self) -> &str {
        "git-log"
    }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> Result<Vec<Proposal<W>>, ObserverError> {
        let since_raw: Option<String> = ctx
            .cache
            .get::<String>("git.last_seen_head_sha")
            .ok()
            .flatten();
        let since = since_raw.as_deref().map(CommitRef::new);

        let commits = self
            .vcs
            .log_since(since.as_ref(), &CommitFilter::default())
            .await
            .map_err(|e| ObserverError::Vcs(Box::new(e)))?;

        let mut out = Vec::new();
        for commit in commits {
            if ctx.cancel.is_cancelled() {
                return Err(ObserverError::Cancelled {
                    name: CompactString::from("git-log"),
                    elapsed_ms: 0,
                });
            }
            let message = if commit.body.is_empty() {
                commit.subject.to_string()
            } else {
                format!("{}\n\n{}", commit.subject, commit.body)
            };
            let Ok(parsed) = parse_commit_message(commit.sha.clone(), &message) else {
                continue;
            };
            let body = match parsed.kind {
                _ if parsed.kind.is_implementation() => {
                    let Some(milestone) = (self.resolver)(&parsed) else {
                        continue;
                    };
                    EventBody::MilestoneShipped {
                        milestone,
                        commit: commit.sha.clone(),
                        commit_kind: parsed.kind,
                        // GitLogObserver walked the commit, so the
                        // commit is visible by construction.
                        status: knotch_kernel::CommitStatus::Verified,
                    }
                }
                CommitKind::Revert => {
                    let Some(original) = parsed.reverts.clone() else { continue };
                    let Some(milestone) = (self.resolver)(&parsed) else {
                        continue;
                    };
                    EventBody::MilestoneReverted {
                        milestone,
                        original,
                        revert: commit.sha.clone(),
                    }
                }
                _ => continue,
            };
            let causation = Causation::new(
                Source::Hook,
                Principal::System { service: CompactString::from("observer") },
                Trigger::Observer { name: CompactString::from("git-log") },
            );
            out.push(Proposal {
                body,
                causation,
                extension: default_extension::<W>(),
                supersedes: None,
            });
            if out.len() >= ctx.budget.max_proposals {
                return Err(ObserverError::BudgetExceeded {
                    name: CompactString::from("git-log"),
                    limit: ctx.budget.max_proposals,
                });
            }
        }
        Ok(out)
    }
}

/// Build a `MilestoneResolver` from a closure that maps a parsed
/// commit to an optional milestone.
pub fn resolver<W, F>(f: F) -> MilestoneResolver<W>
where
    W: WorkflowKind,
    F: Fn(&knotch_vcs::ParsedCommit) -> Option<W::Milestone> + Send + Sync + 'static,
{
    Arc::new(f)
}

fn default_extension<W: WorkflowKind>() -> W::Extension {
    // `W::Extension` must round-trip through serde; the kernel's
    // default "no extension" type `()` produces an empty JSON null.
    // Workflows that require a populated extension must build their
    // own observers rather than using this default.
    serde_json::from_value(serde_json::Value::Null)
        .expect("extension type must deserialize from JSON null")
}
