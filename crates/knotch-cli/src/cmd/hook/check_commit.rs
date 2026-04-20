//! PreToolUse(git commit) → validate milestone against shipped set.

use knotch_agent::{
    HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
};
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let Some(command) = input.bash_command() else {
        return Ok(HookOutput::Continue);
    };
    if !command.trim_start().starts_with("git commit") {
        return Ok(HookOutput::Continue);
    }
    let Some(msg) = knotch_agent::commit::extract_commit_message(command) else {
        return Ok(HookOutput::Continue);
    };
    let root = project_root(&input.cwd);
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        ActiveUnit::Uninitialized | ActiveUnit::NoProject => return Ok(HookOutput::Continue),
    };
    let repo = config.build_repository()?;
    Ok(knotch_agent::commit::check::<ConfigWorkflow, _>(&repo, &unit, &msg).await?)
}
