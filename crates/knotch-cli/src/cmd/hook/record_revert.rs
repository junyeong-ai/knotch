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
    queue::{PostToolContext, post_tool_append},
};
use knotch_kernel::{CommitRef, EventBody, Repository, UnitId, WorkflowKind};
use knotch_workflow::ConfigWorkflow;

use crate::{config::Config, home::user_home};

pub(crate) async fn run(config: &Config, input: HookInput) -> Result<HookOutput> {
    let Some(command) = input.bash_command() else {
        return Ok(HookOutput::Continue);
    };
    let Some(original) = extract_revert_target_from_cmd(command) else {
        return Ok(HookOutput::Continue);
    };
    let Some(revert_sha) = input.bash_response_stdout().and_then(extract_sha_from_stdout) else {
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
    dispatch::<ConfigWorkflow, _>(
        &repo,
        DispatchArgs {
            config,
            root: &root,
            cwd: &input.cwd,
            unit: &unit,
            revert: revert_ref,
            original: original_ref,
            causation,
        },
    )
    .await
}

struct DispatchArgs<'a> {
    config: &'a Config,
    root: &'a std::path::Path,
    cwd: &'a std::path::Path,
    unit: &'a UnitId,
    revert: CommitRef,
    original: CommitRef,
    causation: knotch_kernel::Causation,
}

async fn dispatch<W, R>(repo: &R, args: DispatchArgs<'_>) -> Result<HookOutput>
where
    W: WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
    knotch_kernel::Proposal<W>: serde::Serialize,
{
    // Find the matching MilestoneShipped event so we can name the
    // milestone on the revert event.
    let log = repo.load(args.unit).await?;
    let milestone = log.events().iter().rev().find_map(|evt| match &evt.body {
        EventBody::MilestoneShipped { milestone, commit, .. } if commit == &args.original => {
            Some(milestone.clone())
        }
        _ => None,
    });
    let Some(milestone) = milestone else {
        // Revert targets a commit outside knotch's awareness —
        // non-blocking silent skip.
        tracing::info!(
            original = args.original.as_str(),
            "knotch record-revert: no MilestoneShipped back-reference — skipped"
        );
        return Ok(HookOutput::Continue);
    };
    let proposal = knotch_agent::commit::revert_proposal::<W>(
        args.revert,
        args.original,
        milestone,
        args.causation,
    );
    let queue_dir = args.root.join(".knotch").join("queue");
    let home = user_home().unwrap_or_else(|| args.root.to_path_buf());
    let ctx = PostToolContext {
        queue_dir: &queue_dir,
        queue_config: &args.config.queue,
        home: &home,
        cwd: args.cwd,
        hook_name: "record-revert",
    };
    Ok(post_tool_append::<W, R>(repo, args.unit, proposal, ctx).await?)
}

fn extract_revert_target_from_cmd(cmd: &str) -> Option<String> {
    let rest = cmd.trim_start().strip_prefix("git revert")?;
    rest.split_whitespace().find(|t| !t.starts_with('-')).map(str::to_owned)
}

fn extract_sha_from_stdout(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some(stripped) = line.strip_prefix('[')
            && let Some(end) = stripped.find(']')
            && let Some(token) = stripped[..end].split_whitespace().nth(1)
        {
            return Some(token.to_owned());
        }
    }
    None
}
