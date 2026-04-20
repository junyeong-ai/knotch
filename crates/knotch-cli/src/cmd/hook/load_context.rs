//! SessionStart → inject active-unit context.

use knotch_agent::{HookInput, HookOutput};
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let repo = config.build_repository()?;
    Ok(knotch_agent::session_start::load_context::<ConfigWorkflow, _>(
        Some(&config.root),
        &input.cwd,
        input.session_id.as_str(),
        &repo,
    )
    .await?)
}
