//! PostToolUse(git revert) → append `MilestoneReverted`.
//!
//! Milestone back-reference is resolved by walking the unit's
//! effective log for the prior `MilestoneShipped { commit: original }`
//! event. If no matching milestone exists (e.g. the revert targets a
//! commit outside knotch's awareness), the hook is a silent no-op.

use anyhow::Result;
use compact_str::CompactString;
use knotch_agent::{
    HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
    causation::hook_causation,
};
use knotch_kernel::{CommitRef, EventBody, Repository, UnitId, WorkflowKind};
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> Result<HookOutput> {
    let Some(command) = input.bash_command() else {
        return Ok(HookOutput::Continue);
    };
    if !command.trim_start().starts_with("git revert") {
        return Ok(HookOutput::Continue);
    }
    let Some(original) = extract_revert_target_from_cmd(command) else {
        return Ok(HookOutput::Continue);
    };
    let Some(revert_sha) =
        input.bash_response_stdout().and_then(extract_sha_from_stdout)
    else {
        return Ok(HookOutput::Continue);
    };
    let root = project_root(&input.cwd);
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        _ => return Ok(HookOutput::Continue),
    };
    let causation = hook_causation(&input, "record-revert");
    let original_ref = CommitRef::new(CompactString::from(original));
    let revert_ref = CommitRef::new(CompactString::from(revert_sha));
    let repo = config.build_repository()?;
    dispatch::<ConfigWorkflow, _>(&repo, &unit, revert_ref, original_ref, causation).await
}

async fn dispatch<W, R>(
    repo: &R,
    unit: &UnitId,
    revert: CommitRef,
    original: CommitRef,
    causation: knotch_kernel::Causation,
) -> Result<HookOutput>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    knotch_kernel::Proposal<W>: serde::Serialize,
{
    // Find the matching MilestoneShipped event so we can name the
    // milestone on the revert event.
    let log = repo.load(unit).await?;
    let milestone = log
        .events()
        .iter()
        .rev()
        .find_map(|evt| match &evt.body {
            EventBody::MilestoneShipped { milestone, commit, .. } if commit == &original => {
                Some(milestone.clone())
            }
            _ => None,
        });
    let Some(milestone) = milestone else {
        // Revert targets a commit outside knotch's awareness —
        // non-blocking silent skip.
        tracing::info!(
            original = original.as_str(),
            "knotch record-revert: no MilestoneShipped back-reference — skipped"
        );
        return Ok(HookOutput::Continue);
    };
    Ok(knotch_agent::commit::record_revert::<W, R>(
        repo, unit, revert, original, milestone, causation,
    )
    .await?)
}

fn extract_revert_target_from_cmd(cmd: &str) -> Option<String> {
    let rest = cmd.trim_start().strip_prefix("git revert")?;
    rest.split_whitespace()
        .find(|t| !t.starts_with('-'))
        .map(str::to_owned)
}

fn extract_sha_from_stdout(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some(stripped) = line.strip_prefix('[') {
            if let Some(end) = stripped.find(']') {
                let header = &stripped[..end];
                if let Some(token) = header.split_whitespace().nth(1) {
                    return Some(token.to_owned());
                }
            }
        }
    }
    None
}
