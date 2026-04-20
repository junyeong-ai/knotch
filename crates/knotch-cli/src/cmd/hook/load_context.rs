//! SessionStart — inject active-unit context and record any
//! model switch since the last session.

use knotch_agent::{HookEvent, HookInput, HookOutput, causation::hook_causation};
use knotch_kernel::causation::ModelId;
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let repo = config.build_repository()?;

    // Detect session-boundary model changes. Claude Code stamps
    // the current model on every SessionStart payload, so the
    // detector just reads `input.event.model` — no env-var
    // plumbing, no mid-session gap beyond what Claude Code itself
    // exposes.
    if let HookEvent::SessionStart { model: Some(model), .. } = &input.event
        && !model.is_empty()
    {
        let causation = hook_causation(&input, "load-context");
        let _ = knotch_agent::model::record_switch_if_changed::<ConfigWorkflow, _>(
            &config.root,
            input.session_id.as_str(),
            &repo,
            ModelId(model.clone()),
            causation,
        )
        .await?;
    }

    Ok(knotch_agent::session_start::load_context::<ConfigWorkflow, _>(
        Some(&config.root),
        &input.cwd,
        input.session_id.as_str(),
        &repo,
    )
    .await?)
}
