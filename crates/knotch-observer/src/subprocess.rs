//! Language-neutral observer via subprocess.
//!
//! `SubprocessObserver` spawns an external binary (Python, Node,
//! Bash, Go — anything with stdin/stdout) and treats it as a
//! first-class `Observer<W>`. The wire protocol is JSON over
//! stdin/stdout. This is the path that lets non-Rust adopters
//! keep their observer logic in their native language while the
//! kernel + reconciler stay Rust.
//!
//! ## Wire protocol
//!
//! **Request** — one line of JSON on stdin:
//!
//! ```json
//! {
//!   "unit": "feature-x",
//!   "head": "abc1234",
//!   "taken_at": "2026-04-19T10:00:00Z",
//!   "events": [ /* Event<W> objects */ ],
//!   "budget": { "max_proposals": 128 }
//! }
//! ```
//!
//! **Response** — one line of JSON on stdout:
//!
//! ```json
//! { "proposals": [ /* Proposal<W> objects */ ] }
//! ```
//!
//! **Exit codes:**
//! - `0` — success; stdout holds the proposals line.
//! - `1` — transient failure (reconciler records as `ObserverError::Backend` and retries
//!   on next reconcile).
//! - `2` — permanent failure (same classification — the subprocess contract doesn't need
//!   two retryable levels; reconciler semantics already treat every observer error as
//!   "try again next cycle" for non-determinism reasons).
//! - other — treated as crash; stderr appended to the error message.
//!
//! ## Manifest
//!
//! Observers are declared in `knotch.toml`:
//!
//! ```toml
//! [[observers]]
//! name = "grove-artifact-scanner"
//! binary = "scripts/specs/observers/artifact_observer.py"
//! args = ["--fmt", "knotch"]
//! subscribes = ["phase_completed", "milestone_shipped"]
//! deterministic = true
//! timeout_ms = 10_000
//! ```

use std::{path::PathBuf, process::Stdio, time::Duration};

use compact_str::CompactString;
use knotch_kernel::{Event, Proposal, WorkflowKind};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

use crate::{Observer, context::ObserveContext, error::ObserverError};

/// Declarative observer manifest. Loaded from the `[[observers]]`
/// array in `knotch.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverManifest {
    /// Stable observer id — used as the sort key for deterministic
    /// reconciler merges and as the `Causation::Trigger::Observer`
    /// attribution.
    pub name: CompactString,
    /// Path to the binary. Relative paths resolve against the
    /// project root (same as `state_dir`).
    pub binary: PathBuf,
    /// CLI arguments passed to the binary, in order.
    #[serde(default)]
    pub args: Vec<String>,
    /// Event kind tags the observer subscribes to. Empty = see
    /// every event on the log.
    #[serde(default)]
    pub subscribes: Vec<CompactString>,
    /// Whether running this observer twice against the same log
    /// state yields the same proposals. Non-deterministic observers
    /// still run, but the reconciler emits a `tracing::warn` so
    /// operators notice that replay won't be stable.
    #[serde(default = "default_deterministic")]
    pub deterministic: bool,
    /// Soft timeout in milliseconds. Reconciler uses this as
    /// `tokio::time::timeout`.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

const fn default_deterministic() -> bool {
    true
}

const fn default_timeout_ms() -> u64 {
    30_000
}

/// Errors specific to subprocess observer construction. Per-invocation
/// failures surface through [`ObserverError`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SubprocessError {
    /// The declared binary does not exist at construction time.
    #[error("observer `{name}` binary not found at {path:?}")]
    BinaryMissing {
        /// Manifest name.
        name: CompactString,
        /// Declared binary path.
        path: PathBuf,
    },
}

/// Subprocess-backed observer. Construct once at CLI startup, pass
/// an `Arc` into the reconciler.
#[derive(Debug)]
pub struct SubprocessObserver<W: WorkflowKind> {
    manifest: ObserverManifest,
    _phantom: std::marker::PhantomData<fn() -> W>,
}

impl<W: WorkflowKind> SubprocessObserver<W> {
    /// Construct a `SubprocessObserver` from a manifest. Validates
    /// that the binary path exists so configuration mistakes surface
    /// at startup, not on the first reconcile.
    ///
    /// # Errors
    /// Returns `SubprocessError::BinaryMissing` when the declared
    /// path does not exist.
    pub fn new(manifest: ObserverManifest) -> Result<Self, SubprocessError> {
        if !manifest.binary.exists() {
            return Err(SubprocessError::BinaryMissing {
                name: manifest.name.clone(),
                path: manifest.binary.clone(),
            });
        }
        Ok(Self { manifest, _phantom: std::marker::PhantomData })
    }

    /// Borrow the manifest — exposed for `knotch doctor` and CLI
    /// diagnostics that enumerate loaded observers.
    #[must_use]
    pub fn manifest(&self) -> &ObserverManifest {
        &self.manifest
    }
}

/// Wire-level request payload — what the subprocess reads on stdin.
#[derive(Debug, Serialize)]
#[serde(bound(serialize = "Event<W>: Serialize"))]
struct Request<'a, W: WorkflowKind> {
    unit: &'a str,
    head: &'a str,
    taken_at: String,
    events: Vec<&'a Event<W>>,
    budget: BudgetWire,
}

#[derive(Debug, Serialize)]
struct BudgetWire {
    max_proposals: usize,
}

/// Wire-level response payload — what the subprocess writes on stdout.
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "Proposal<W>: DeserializeOwned"))]
struct Response<W: WorkflowKind> {
    proposals: Vec<Proposal<W>>,
}

impl<W> Observer<W> for SubprocessObserver<W>
where
    W: WorkflowKind,
    Event<W>: Serialize,
    Proposal<W>: DeserializeOwned,
{
    fn name(&self) -> &str {
        self.manifest.name.as_str()
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.manifest.timeout_ms)
    }

    async fn observe<'ctx>(
        &'ctx self,
        ctx: &'ctx ObserveContext<'ctx, W>,
    ) -> Result<Vec<Proposal<W>>, ObserverError> {
        // Cancellation short-circuit: if the reconciler has already
        // tripped the token, skip the fork-exec entirely.
        if ctx.cancel.is_cancelled() {
            return Err(ObserverError::Cancelled {
                name: self.manifest.name.clone(),
                elapsed_ms: 0,
            });
        }

        let events: Vec<&Event<W>> = if self.manifest.subscribes.is_empty() {
            ctx.log.events().iter().collect()
        } else {
            let want: std::collections::HashSet<&str> =
                self.manifest.subscribes.iter().map(CompactString::as_str).collect();
            ctx.log.events().iter().filter(|e| want.contains(e.body.kind_tag())).collect()
        };

        let req = Request::<W> {
            unit: ctx.unit.as_str(),
            head: ctx.head,
            taken_at: ctx.taken_at.to_string(),
            events,
            budget: BudgetWire { max_proposals: ctx.budget.max_proposals },
        };

        let mut cmd = Command::new(&self.manifest.binary);
        cmd.args(&self.manifest.args);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|source| ObserverError::Backend(Box::new(source)))?;

        let stdin_payload =
            serde_json::to_vec(&req).map_err(|source| ObserverError::Backend(Box::new(source)))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&stdin_payload)
                .await
                .map_err(|source| ObserverError::Backend(Box::new(source)))?;
            stdin.shutdown().await.map_err(|source| ObserverError::Backend(Box::new(source)))?;
        }

        // Read stdout + stderr in parallel so a chatty stderr
        // doesn't deadlock the pipe.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut s) = stdout {
                s.read_to_end(&mut buf).await.ok();
            }
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut s) = stderr {
                s.read_to_end(&mut buf).await.ok();
            }
            buf
        });

        let status =
            child.wait().await.map_err(|source| ObserverError::Backend(Box::new(source)))?;

        let stdout_bytes = stdout_task.await.unwrap_or_default();
        let stderr_bytes = stderr_task.await.unwrap_or_default();

        if !status.success() {
            let stderr_text = String::from_utf8_lossy(&stderr_bytes).into_owned();
            let message = format!(
                "observer `{}` exited {code}: {stderr}",
                self.manifest.name,
                code = status.code().map_or_else(|| "signal".to_owned(), |c| c.to_string()),
                stderr = stderr_text.trim(),
            );
            return Err(ObserverError::Backend(Box::<dyn std::error::Error + Send + Sync>::from(
                message,
            )));
        }

        if stdout_bytes.is_empty() {
            return Ok(Vec::new());
        }

        let response: Response<W> = serde_json::from_slice(&stdout_bytes)
            .map_err(|source| ObserverError::Backend(Box::new(source)))?;

        if response.proposals.len() > ctx.budget.max_proposals {
            return Err(ObserverError::BudgetExceeded {
                name: self.manifest.name.clone(),
                limit: ctx.budget.max_proposals,
            });
        }

        Ok(response.proposals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_default_deterministic() {
        let toml_src = r#"
            name = "grove-artifact"
            binary = "/tmp/dummy"
        "#;
        let m: ObserverManifest = toml::from_str(toml_src).unwrap();
        assert_eq!(m.name, "grove-artifact");
        assert!(m.deterministic);
        assert_eq!(m.timeout_ms, 30_000);
        assert!(m.subscribes.is_empty());
    }

    #[test]
    fn new_rejects_missing_binary() {
        use knotch_workflow::ConfigWorkflow;
        let manifest = ObserverManifest {
            name: "ghost".into(),
            binary: PathBuf::from("/nonexistent/ghost-observer"),
            args: vec![],
            subscribes: vec![],
            deterministic: true,
            timeout_ms: 1_000,
        };
        let err = SubprocessObserver::<ConfigWorkflow>::new(manifest).unwrap_err();
        assert!(matches!(err, SubprocessError::BinaryMissing { .. }));
    }
}
