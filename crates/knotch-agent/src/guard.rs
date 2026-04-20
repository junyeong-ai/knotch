//! `PreToolUse(git push --force | reset --hard | branch -D | ...)` —
//! blocks destructive history-rewriting git operations against
//! shipped or terminal-status units. Exposed as the CLI
//! `knotch hook guard-rewrite` subcommand.

use knotch_kernel::{
    Repository, UnitId, WorkflowKind,
    project::{current_status, shipped_milestones},
};

use crate::{error::HookError, output::HookOutput};

/// Inspect the active unit and block the command when:
///
/// 1. the unit's current status is terminal (per [`WorkflowKind::is_terminal_status`]),
///    or
/// 2. the unit has any shipped milestones not yet reverted.
///
/// Otherwise the command proceeds.
pub async fn rewrite<W, R>(repo: &R, unit: &UnitId, cmd: &str) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    R: Repository<W>,
{
    let log = repo.load(unit).await?;
    if let Some(status) = current_status(&log) {
        if repo.workflow().is_terminal_status(&status) {
            return Ok(HookOutput::block(format!(
                "Destructive command `{cmd}` blocked: unit `{}` is in terminal status `{}`.",
                unit.as_str(),
                status.as_str()
            )));
        }
    }
    let shipped = shipped_milestones(&log);
    if !shipped.is_empty() {
        return Ok(HookOutput::block(format!(
            "Destructive command `{cmd}` blocked: unit `{}` has {} shipped milestone(s). \
             Supersede or revert them before rewriting history.",
            unit.as_str(),
            shipped.len()
        )));
    }
    Ok(HookOutput::Continue)
}
