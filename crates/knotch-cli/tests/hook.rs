//! `knotch hook <subcommand>` golden paths.
//!
//! Exit-code contract per `.claude/rules/hook-integration.md`:
//! - `Continue` → exit 0, empty stdout.
//! - `Context(s)` → exit 0, JSON with `additionalContext`.
//! - `Block { reason }` → exit 2, reason on stderr.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;

/// Minimal `knotch.toml` body — picks the vibe preset.
const VIBE_CONFIG: &str = "state_dir = \"state\"\npreset = \"vibe\"\n";

fn setup() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("knotch.toml"), VIBE_CONFIG).expect("write knotch.toml");
    std::fs::create_dir_all(tmp.path().join(".knotch")).expect("mkdir .knotch");
    std::fs::create_dir_all(tmp.path().join("state")).expect("mkdir state");
    tmp
}

fn session_start_input(cwd: &std::path::Path) -> String {
    json!({
        "session_id": "11111111-1111-7111-8111-111111111111",
        "cwd": cwd.display().to_string(),
        "hook_event_name": "SessionStart",
        "source": "startup"
    })
    .to_string()
}

fn pretool_bash_input(cwd: &std::path::Path, command: &str) -> String {
    json!({
        "session_id": "22222222-2222-7222-8222-222222222222",
        "cwd": cwd.display().to_string(),
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command }
    })
    .to_string()
}

#[test]
fn load_context_on_uninitialized_project_yields_continue_message() {
    let tmp = setup();
    Command::cargo_bin("knotch")
        .unwrap()
        .arg("--root")
        .arg(tmp.path())
        .args(["hook", "load-context"])
        .write_stdin(session_start_input(tmp.path()))
        .assert()
        .success()
        .stdout(predicate::str::contains("additionalContext"));
}

#[test]
fn load_context_outside_any_project_is_silent_success() {
    let tmp = tempfile::tempdir().unwrap(); // no knotch.toml
    Command::cargo_bin("knotch")
        .unwrap()
        .arg("--root")
        .arg(tmp.path())
        .args(["hook", "load-context"])
        .write_stdin(session_start_input(tmp.path()))
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn check_commit_passes_through_non_bash_commands() {
    let tmp = setup();
    Command::cargo_bin("knotch")
        .unwrap()
        .arg("--root")
        .arg(tmp.path())
        .args(["hook", "check-commit"])
        .write_stdin(pretool_bash_input(tmp.path(), "ls -la"))
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn check_commit_without_active_unit_continues() {
    let tmp = setup();
    // active.toml is absent — hook must not block.
    Command::cargo_bin("knotch")
        .unwrap()
        .arg("--root")
        .arg(tmp.path())
        .args(["hook", "check-commit"])
        .write_stdin(pretool_bash_input(
            tmp.path(),
            "git commit -m \"feat: add sso\"",
        ))
        .assert()
        .success();
}

#[test]
fn guard_rewrite_on_non_destructive_command_continues() {
    let tmp = setup();
    Command::cargo_bin("knotch")
        .unwrap()
        .arg("--root")
        .arg(tmp.path())
        .args(["hook", "guard-rewrite"])
        .write_stdin(pretool_bash_input(tmp.path(), "git status"))
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}
