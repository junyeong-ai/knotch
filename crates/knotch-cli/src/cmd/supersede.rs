//! `knotch supersede` — preset-bound command (informative in Phase 8).

use clap::Args as ClapArgs;

use crate::{cmd::OutputMode, config::Config};

/// `knotch supersede` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Unit owning the event to supersede.
    pub unit: Option<String>,
    /// Target event id.
    #[arg(long)]
    pub event: Option<String>,
    /// Non-empty rationale.
    #[arg(long)]
    pub rationale: Option<String>,
}

/// Run the supersede command.
///
/// # Errors
/// Always returns an error in Phase 8 — append requires a preset
/// binding that ships in Phase 9.
pub(crate) async fn run(_config: &Config, out: OutputMode, _args: Args) -> anyhow::Result<()> {
    if out.is_json() {
        println!(
            "{}",
            serde_json::json!({
                "error": "preset-required",
                "phase": 9,
                "message": "supersede appends an EventSuperseded which requires a WorkflowKind \
                            binding — use a preset crate",
            })
        );
    } else {
        eprintln!(
            "knotch supersede appends an EventSuperseded event; this requires a \
             WorkflowKind binding that ships with preset crates in Phase 9."
        );
    }
    anyhow::bail!("supersede: preset required");
}
