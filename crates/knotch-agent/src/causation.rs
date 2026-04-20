//! Construct `Causation` values for hook invocations.
//!
//! Canonical attribution for every hook-emitted event:
//!
//! - `source = Source::Hook`
//! - `principal = Principal::Agent { agent_id, model }` — `agent_id` comes from the
//!   hook's `session_id`, `model` comes from `KNOTCH_MODEL` env var (falls back to
//!   `"unknown"`).
//! - `trigger = Trigger::GitHook(<subcommand>)` — the `knotch hook <name>` wrapper.
//! - `session` — set when `session_id` parses as a UUID; otherwise omitted.

use compact_str::CompactString;
use knotch_kernel::{
    Causation,
    causation::{AgentId, ModelId, Principal, SessionId, Source, Trigger},
};

use crate::input::HookInput;

/// Build a `Causation` for a hook-emitted event.
///
/// `agent_id` resolution prefers the event-supplied value
/// ([`HookEvent::agent_id`], currently `SubagentStop` only) and
/// falls back to `session_id` as best-effort attribution.
#[must_use]
pub fn hook_causation(input: &HookInput, subcommand: &str) -> Causation {
    let model = std::env::var("KNOTCH_MODEL").unwrap_or_else(|_| "unknown".to_owned());
    let agent_id =
        input.event.agent_id().map(CompactString::from).unwrap_or_else(|| input.session_id.clone());
    let principal = Principal::Agent {
        agent_id: AgentId(agent_id),
        model: ModelId(CompactString::from(model)),
    };
    Causation::new(
        Source::Hook,
        principal,
        Trigger::GitHook { name: CompactString::from(subcommand) },
    )
    .with_session(SessionId::parse(input.session_id.as_str()))
}
