//! End-to-end SubprocessObserver test. Spawns a real shell script
//! that implements the wire protocol and confirms proposals round-trip.
//!
//! Gated to `#[cfg(unix)]` because the script is a POSIX shebang
//! (`#!/bin/sh`) and the chmod-to-755 setup uses
//! `std::os::unix::fs::PermissionsExt`. Windows integration lands
//! separately (a `cmd.exe` / PowerShell shim) if demand warrants —
//! `SubprocessObserver` itself is cross-platform at the Rust level.

#![cfg(unix)]
#![allow(missing_docs)]

use std::{path::PathBuf, sync::Arc};

use knotch_kernel::{
    Log, Scope, UnitId, WorkflowKind,
    event::{ArtifactList, EventBody},
    repository::ResumeCache,
};
use knotch_observer::{
    ObserveBudget, ObserveContext, Observer, ObserverManifest, SubprocessObserver,
};
use knotch_workflow::ConfigWorkflow;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Produce a shell script at `dir/name` that reads the JSON request
/// on stdin, ignores the content, and emits the provided response
/// line on stdout.
fn script_emitting(dir: &std::path::Path, name: &str, stdout_line: &str) -> PathBuf {
    let path = dir.join(name);
    let body = format!(
        "#!/bin/sh\nset -e\n# drain stdin so the parent's write_all doesn't block\ncat >/dev/null\ncat <<'KNOTCH_EOF'\n{stdout_line}\nKNOTCH_EOF\n"
    );
    std::fs::write(&path, body).expect("write script");
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn minimal_context<'a, W: WorkflowKind>(
    workflow_log: Arc<Log<W>>,
    unit: &'a UnitId,
    cache: &'a ResumeCache,
    cancel: &'a CancellationToken,
    head: &'a str,
) -> ObserveContext<'a, W> {
    ObserveContext {
        unit,
        log: workflow_log,
        head,
        cache,
        taken_at: jiff::Timestamp::now(),
        cancel,
        budget: ObserveBudget::default(),
    }
}

#[tokio::test]
async fn observer_returning_empty_proposals_succeeds() {
    let dir = TempDir::new().unwrap();
    let bin = script_emitting(dir.path(), "empty.sh", r#"{"proposals":[]}"#);

    let manifest = ObserverManifest {
        name: "empty".into(),
        binary: bin,
        args: vec![],
        subscribes: vec![],
        deterministic: true,
        timeout_ms: 10_000,
    };
    let observer = SubprocessObserver::<ConfigWorkflow>::new(manifest).expect("binary exists");

    let unit = UnitId::try_new("u").unwrap();
    let log = Arc::new(Log::<ConfigWorkflow>::from_events(unit.clone(), vec![]));
    let cache = ResumeCache::new();
    let cancel = CancellationToken::new();
    let ctx = minimal_context(log, &unit, &cache, &cancel, "abc1234");

    let proposals = observer.observe(&ctx).await.expect("run script");
    assert_eq!(proposals.len(), 0);
}

#[tokio::test]
async fn observer_non_zero_exit_surfaces_as_backend_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fail.sh");
    std::fs::write(&path, "#!/bin/sh\ncat >/dev/null\necho 'something broke' >&2\nexit 1\n")
        .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();

    let manifest = ObserverManifest {
        name: "fail".into(),
        binary: path,
        args: vec![],
        subscribes: vec![],
        deterministic: true,
        timeout_ms: 10_000,
    };
    let observer = SubprocessObserver::<ConfigWorkflow>::new(manifest).expect("binary exists");

    let unit = UnitId::try_new("u").unwrap();
    let log = Arc::new(Log::<ConfigWorkflow>::from_events(unit.clone(), vec![]));
    let cache = ResumeCache::new();
    let cancel = CancellationToken::new();
    let ctx = minimal_context(log, &unit, &cache, &cancel, "abc1234");

    let err = observer.observe(&ctx).await.expect_err("exit 1 is an error");
    let msg = err.to_string();
    assert!(
        msg.contains("observer backend failure") || msg.contains("fail"),
        "unexpected error message: {msg}",
    );
}

#[tokio::test]
async fn observer_receives_events_filtered_by_subscription() {
    // The script echoes its stdin to a sidechannel file, so we can
    // inspect what the parent wrote. Uses file mkstemp to avoid
    // clobbering between parallel test runs.
    let dir = TempDir::new().unwrap();
    let sidechannel = dir.path().join("seen.json");
    let path = dir.path().join("echo.sh");
    let body = format!(
        "#!/bin/sh\nset -e\ncat > '{}'\ncat <<'KNOTCH_EOF'\n{{\"proposals\":[]}}\nKNOTCH_EOF\n",
        sidechannel.display()
    );
    std::fs::write(&path, body).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();

    let manifest = ObserverManifest {
        name: "echo".into(),
        binary: path,
        args: vec![],
        subscribes: vec!["phase_completed".into()],
        deterministic: true,
        timeout_ms: 10_000,
    };
    let observer = SubprocessObserver::<ConfigWorkflow>::new(manifest).expect("binary exists");

    // Build a log with UnitCreated + one PhaseCompleted + one
    // MilestoneShipped-unrelated event. Subscription filter should
    // only pass `phase_completed`.
    let unit = UnitId::try_new("u").unwrap();
    let causation = knotch_kernel::Causation::new(
        knotch_kernel::causation::Source::Cli,
        knotch_kernel::causation::Trigger::Command { name: "test".into() },
    );
    let specify = ConfigWorkflow::canonical().parse_phase("specify").expect("specify phase");
    let events = vec![
        knotch_kernel::Event {
            id: knotch_kernel::EventId::new_v7(),
            at: jiff::Timestamp::now(),
            causation: causation.clone(),
            extension: knotch_workflow::DynamicExtension::default(),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        },
        knotch_kernel::Event {
            id: knotch_kernel::EventId::new_v7(),
            at: jiff::Timestamp::now(),
            causation: causation.clone(),
            extension: knotch_workflow::DynamicExtension::default(),
            body: EventBody::PhaseCompleted { phase: specify, artifacts: ArtifactList::default() },
            supersedes: None,
        },
    ];
    let log = Arc::new(Log::<ConfigWorkflow>::from_events(unit.clone(), events));
    let cache = ResumeCache::new();
    let cancel = CancellationToken::new();
    let ctx = minimal_context(log, &unit, &cache, &cancel, "deadbee");

    observer.observe(&ctx).await.expect("script runs");

    let raw = std::fs::read_to_string(&sidechannel).expect("sidechannel populated");
    let seen: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    let events_out = seen.get("events").and_then(|v| v.as_array()).expect("events array");
    assert_eq!(events_out.len(), 1, "subscription filter must drop non-phase-completed events");
    let first = &events_out[0];
    let kind = first.pointer("/body/type").and_then(|v| v.as_str()).expect("body.type");
    assert_eq!(kind, "phase_completed");
}
