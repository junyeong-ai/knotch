//! SessionStart — inject active-unit context and record any
//! model switch since the last session.

use knotch_agent::{HookInput, HookOutput, causation::hook_causation};
use knotch_kernel::causation::ModelId;
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let repo = config.build_repository()?;

    // Detect mid-session model changes before emitting the
    // context injection. Silent no-op on unset `$KNOTCH_MODEL`:
    // without a known current value we cannot assert a change.
    if let Ok(current) = std::env::var("KNOTCH_MODEL")
        && !current.is_empty()
    {
        let causation = hook_causation(&input, "load-context");
        let _ = knotch_agent::model::record_switch_if_changed::<ConfigWorkflow, _>(
            &config.root,
            input.session_id.as_str(),
            &repo,
            ModelId(current.into()),
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
