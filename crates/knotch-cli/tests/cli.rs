//! End-to-end CLI tests driving the real `knotch` binary.

use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::str;
use serde_json::Value;

fn bin() -> Command {
    Command::cargo_bin("knotch").expect("binary built")
}

#[test]
fn init_creates_config_and_state_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin()
        .current_dir(dir.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(str::contains("initialized knotch workspace"));

    let cfg = dir.path().join("knotch.toml");
    assert!(cfg.exists(), "knotch.toml not created");
    let state = dir.path().join("state");
    assert!(state.is_dir(), "state dir not created");
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["init"])
        .assert()
        .failure()
        .stderr(str::contains("already exists"));
}

#[test]
fn init_force_overwrites() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin().current_dir(dir.path()).args(["init", "--force"]).assert().success();
}

#[test]
fn unit_init_emits_unit_created_event_with_default_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["unit", "init", "feat-auth"])
        .assert()
        .success()
        .stdout(str::contains("scope: standard"))
        .stdout(str::contains("first event: UnitCreated"));

    // `--json log` emits the raw JSONL stream; parse and confirm the
    // first body carries `UnitCreated { scope: "standard" }`.
    let output =
        bin().current_dir(dir.path()).args(["--json", "log", "feat-auth"]).output().expect("run");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let first = &parsed.as_array().expect("array")[0];
    assert_eq!(first["body"]["type"], "unit_created");
    assert_eq!(first["body"]["scope"], "standard");
}

#[test]
fn unit_init_honours_explicit_scope_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["unit", "init", "hotfix-bug", "--scope", "tiny"])
        .assert()
        .success()
        .stdout(str::contains("scope: tiny"));

    let output =
        bin().current_dir(dir.path()).args(["--json", "log", "hotfix-bug"]).output().expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(parsed[0]["body"]["scope"], "tiny");
}

#[test]
fn unit_init_rejects_when_unit_already_has_anchor() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin().current_dir(dir.path()).args(["unit", "init", "feat-x"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["unit", "init", "feat-x"])
        .assert()
        .failure()
        .stderr(str::contains("already initialized"));
}

#[test]
fn log_reads_seeded_jsonl() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();

    let unit_dir = dir.path().join("state").join("unit-1");
    fs::create_dir_all(&unit_dir).expect("unit dir");
    let log_path = unit_dir.join("log.jsonl");
    write_fixture_log(&log_path);

    bin()
        .current_dir(dir.path())
        .args(["log", "unit-1"])
        .assert()
        .success()
        .stdout(str::contains("phase_completed"))
        .stdout(str::contains("(2 event(s))"));
}

#[test]
fn log_json_mode_emits_parseable_array() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();

    let unit_dir = dir.path().join("state").join("unit-json");
    fs::create_dir_all(&unit_dir).expect("unit dir");
    write_fixture_log(&unit_dir.join("log.jsonl"));

    let output =
        bin().current_dir(dir.path()).args(["--json", "log", "unit-json"]).output().expect("run");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let arr = parsed.as_array().expect("array");
    assert_eq!(arr.len(), 2);
}

#[test]
fn show_brief_renders_active_unit() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init", "--demo"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["show", "demo", "--format", "brief"])
        .assert()
        .success()
        .stdout(str::contains("demo"))
        .stdout(str::contains("phase="));
}

#[test]
fn show_summary_is_the_default_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init", "--demo"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["show", "demo"])
        .assert()
        .success()
        .stdout(str::contains("unit:"))
        .stdout(str::contains("current phase:"))
        .stdout(str::contains("last completed:"))
        .stdout(str::contains("events recorded:"));
}

#[test]
fn show_json_includes_last_completed_phase() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init", "--demo"]).assert().success();

    let output = bin()
        .current_dir(dir.path())
        .args(["show", "demo", "--format", "json"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    // --demo seeds a PhaseCompleted on `specify`; assert both
    // projections surface together.
    assert_eq!(parsed["last_completed_phase"], "specify");
    assert!(parsed["current_phase"].is_string());
}

#[test]
fn doctor_reports_clean_after_init() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["doctor"])
        .assert()
        .success()
        .stdout(str::contains("[ OK ] knotch.toml"));
}

#[test]
fn doctor_warns_on_unit_missing_unit_created_anchor() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();

    // Seed a legacy-shaped unit whose log has events but no
    // `UnitCreated` anchor — the exact shape a pre-C5 `unit init`
    // or Rust-API-direct caller would produce.
    let unit_dir = dir.path().join("state").join("legacy-unit");
    fs::create_dir_all(&unit_dir).expect("unit dir");
    write_fixture_log(&unit_dir.join("log.jsonl"));

    bin()
        .current_dir(dir.path())
        .args(["doctor"])
        .assert()
        .success()
        .stdout(str::contains("[WARN] anchors"))
        .stdout(str::contains("legacy-unit"))
        .stdout(str::contains("missing UnitCreated"));
}

#[test]
fn reconcile_drains_empty_queue_cleanly() {
    // Empty queue = no-op drain, zero pruned, exit 0.
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["reconcile"])
        .assert()
        .success()
        .stdout(str::contains("drained:  0"));
}

#[test]
fn reconcile_prune_flag_reports_zero_when_queue_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["reconcile", "--prune"])
        .assert()
        .success()
        .stdout(str::contains("pruned:   0 (all remaining)"));
}

#[test]
fn reconcile_reports_no_observers_when_none_declared() {
    // `knotch init` writes the canonical `knotch.toml` with an empty
    // `[[observers]]` section — `knotch reconcile` should run the
    // drain + observer pass, find zero manifests, and emit the
    // "no [[observers]] declared" signal.
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["reconcile"])
        .assert()
        .success()
        .stdout(str::contains("observers: no [[observers]] declared"));
}

#[test]
fn reconcile_queue_only_flag_skips_observer_pass() {
    // `--queue-only` bypasses subprocess observer dispatch even when
    // manifests are declared. Here we don't even declare any, but the
    // flag should short-circuit the branch that would otherwise emit
    // "no [[observers]] declared".
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin()
        .current_dir(dir.path())
        .args(["reconcile", "--queue-only"])
        .assert()
        .success()
        .stdout(str::contains("observers: skipped (--queue-only)"));
}

#[test]
fn supersede_records_event_superseded_through_config_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin().current_dir(dir.path()).args(["unit", "init", "feat-s"]).assert().success();

    // Grab the UnitCreated event id via --json log.
    let output =
        bin().current_dir(dir.path()).args(["--json", "log", "feat-s"]).output().expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let event_id = parsed[0]["id"].as_str().expect("id").to_owned();

    bin()
        .current_dir(dir.path())
        .args([
            "supersede",
            "feat-s",
            event_id.as_str(),
            "UnitCreated scope was wrong — retry via repair flow",
        ])
        .assert()
        .success()
        .stdout(str::contains("accepted"));

    // Second UnitCreated should NOT be appended — but the
    // EventSuperseded we just recorded must be present.
    let output =
        bin().current_dir(dir.path()).args(["--json", "log", "feat-s"]).output().expect("run");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let events = parsed.as_array().expect("array");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["body"]["type"], "event_superseded");
    assert_eq!(events[1]["body"]["target"], event_id);
}

#[test]
fn supersede_rejects_invalid_event_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    bin().current_dir(dir.path()).args(["init"]).assert().success();
    bin().current_dir(dir.path()).args(["unit", "init", "feat-s"]).assert().success();

    bin()
        .current_dir(dir.path())
        .args(["supersede", "feat-s", "not-a-uuid", "rationale that is long enough"])
        .assert()
        .failure()
        .stderr(str::contains("invalid event id"));
}

#[test]
fn completions_emits_script() {
    bin().args(["completions", "bash"]).assert().success().stdout(str::contains("complete"));
}

fn write_fixture_log(path: &Path) {
    let header =
        r#"{"kind":"__header__","schema_version":1,"workflow":"demo","fingerprint_salt":""}"#;
    let evt1 = r#"{"id":"01900000-0000-7000-8000-000000000001","at":"2026-04-19T10:00:00Z","causation":{"source":"cli","principal":{"kind":"system","service":"demo"},"trigger":{"kind":"manual"}},"extension":null,"body":{"type":"phase_completed","phase":"specify","artifacts":[]},"supersedes":null}"#;
    let evt2 = r#"{"id":"01900000-0000-7000-8000-000000000002","at":"2026-04-19T10:01:00Z","causation":{"source":"cli","principal":{"kind":"system","service":"demo"},"trigger":{"kind":"manual"}},"extension":null,"body":{"type":"status_transitioned","target":"in_review","forced":false,"rationale":null},"supersedes":null}"#;
    let body = format!("{header}\n{evt1}\n{evt2}\n");
    fs::write(path, body).expect("write fixture log");
}
