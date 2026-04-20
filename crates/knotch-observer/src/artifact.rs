//! `ArtifactObserver` — emits `PhaseCompleted` events when all of a
//! phase's required artifacts exist on the filesystem.
//!
//! The observer is workflow-agnostic; the caller supplies a
//! `PhaseScanner` that yields, for a given unit directory, which
//! phases' artifact contracts are currently satisfied.

use std::{path::PathBuf, sync::Arc};

use compact_str::CompactString;
use knotch_kernel::{
    Causation, Proposal, WorkflowKind,
    causation::{Principal, Source, Trigger},
    event::{ArtifactList, EventBody},
    project::effective_events,
};

use crate::{FsView, ObserveContext, Observer, ObserverError, StdFsView};

/// Function producing the phases currently considered "artifacts
/// present" for the supplied unit directory. Callers typically
/// walk `<unit_dir>/<phase_id>/<required-file>` or consult a
/// workflow-specific convention.
pub type PhaseScanner<W> =
    Arc<dyn Fn(&dyn FsView, &std::path::Path) -> Vec<ArtifactScan<W>> + Send + Sync + 'static>;

/// One scan result — a phase whose artifacts are present along with
/// the actual paths they resolved to.
#[derive(Debug, Clone)]
pub struct ArtifactScan<W: WorkflowKind> {
    /// Phase whose contract is satisfied.
    pub phase: W::Phase,
    /// Paths that satisfied the contract (recorded on the event).
    pub artifacts: ArtifactList,
}

/// Artifact observer.
pub struct ArtifactObserver<W: WorkflowKind> {
    unit_root: PathBuf,
    fs: Arc<dyn FsView>,
    scanner: PhaseScanner<W>,
}

impl<W: WorkflowKind> ArtifactObserver<W> {
    /// Build with the default `StdFsView`. The unit id is read from
    /// `ObserveContext` at call time.
    pub fn new(unit_root: PathBuf, scanner: PhaseScanner<W>) -> Self {
        Self { unit_root, fs: Arc::new(StdFsView), scanner }
    }

    /// Override the filesystem view (e.g. with an in-memory fake).
    #[must_use]
    pub fn with_fs(mut self, fs: Arc<dyn FsView>) -> Self {
        self.fs = fs;
        self
    }
}

impl<W: WorkflowKind> Observer<W> for ArtifactObserver<W> {
    fn name(&self) -> &str {
        "artifact"
    }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> Result<Vec<Proposal<W>>, ObserverError> {
        let scans = (self.scanner)(self.fs.as_ref(), &self.unit_root);
        let already: Vec<W::Phase> = effective_events(&ctx.log)
            .iter()
            .filter_map(|evt| match &evt.body {
                EventBody::PhaseCompleted { phase, .. } | EventBody::PhaseSkipped { phase, .. } => {
                    Some(phase.clone())
                }
                _ => None,
            })
            .collect();

        let mut out = Vec::new();
        for scan in scans {
            if ctx.cancel.is_cancelled() {
                return Err(ObserverError::Cancelled {
                    name: CompactString::from("artifact"),
                    elapsed_ms: 0,
                });
            }
            if already.contains(&scan.phase) {
                continue;
            }
            let causation = Causation::new(
                Source::Hook,
                Principal::System { service: CompactString::from("observer") },
                Trigger::Observer { name: CompactString::from("artifact") },
            );
            out.push(Proposal {
                body: EventBody::PhaseCompleted { phase: scan.phase, artifacts: scan.artifacts },
                causation,
                extension: serde_json::from_value(serde_json::Value::Null)
                    .expect("extension must deserialize from null"),
                supersedes: None,
            });
            if out.len() >= ctx.budget.max_proposals {
                return Err(ObserverError::BudgetExceeded {
                    name: CompactString::from("artifact"),
                    limit: ctx.budget.max_proposals,
                });
            }
        }
        Ok(out)
    }
}

/// Build a `PhaseScanner` from a closure.
pub fn scanner<W, F>(f: F) -> PhaseScanner<W>
where
    W: WorkflowKind,
    F: Fn(&dyn FsView, &std::path::Path) -> Vec<ArtifactScan<W>> + Send + Sync + 'static,
{
    Arc::new(f)
}
