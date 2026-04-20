//! Orphan logging — for hook invocations with no active unit.
//!
//! Writes to `<home>/.knotch/orphan.log` so operators can inspect
//! when a hook fired in a directory that is not yet a knotch
//! project, or where `.knotch/active.toml` is empty.
//!
//! Failures to write the log are silent — orphan logging is strictly
//! advisory and must never block a tool call.

use std::{io::Write, path::Path};

/// Append a one-line orphan record. All errors are swallowed.
pub fn log_orphan(home: &Path, event: &str, cwd: &Path, reason: &str) {
    let dir = home.join(".knotch");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let log_path = dir.join("orphan.log");
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) else {
        return;
    };
    let ts = jiff::Timestamp::now();
    let _ = writeln!(file, "{ts} {event} reason={reason} cwd={}", cwd.display());
}
