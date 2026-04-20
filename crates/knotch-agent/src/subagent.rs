//! `SubagentStop` bookkeeping.
//!
//! Records the transcript path and last-message under
//! `.knotch/subagents/<agent-id>.json` so later `Causation` values
//! can reference the subagent run without inflating the main event
//! log. No events are emitted by this hook directly.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{error::HookError, output::HookOutput};

/// On-disk subagent record.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubagentRecord {
    /// Harness-assigned agent id (e.g. `agent-abc123`).
    pub agent_id: String,
    /// Agent type (`Explore`, `Plan`, or a custom name).
    pub agent_type: String,
    /// Absolute path to the subagent's transcript JSONL, if present.
    pub transcript_path: Option<String>,
    /// Last assistant message text, if the harness provided one.
    pub last_message: Option<String>,
    /// ISO-8601 timestamp when the subagent stopped.
    pub stopped_at: String,
}

/// Write a subagent record atomically. Returns
/// [`HookOutput::Continue`] on success.
pub fn record(
    project_root: &Path,
    agent_id: &str,
    agent_type: &str,
    transcript_path: Option<&Path>,
    last_message: Option<&str>,
) -> Result<HookOutput, HookError> {
    let dir = project_root.join(".knotch").join("subagents");
    std::fs::create_dir_all(&dir)?;
    let record = SubagentRecord {
        agent_id: agent_id.to_owned(),
        agent_type: agent_type.to_owned(),
        transcript_path: transcript_path.map(|p| p.to_string_lossy().into_owned()),
        last_message: last_message.map(str::to_owned),
        stopped_at: jiff::Timestamp::now().to_string(),
    };
    let path = dir.join(format!("{agent_id}.json"));
    let body = serde_json::to_vec_pretty(&record)?;
    crate::atomic::write(&path, &body)?;
    Ok(HookOutput::Continue)
}
