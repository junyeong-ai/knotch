//! Construct `Causation` values for hook invocations.
//!
//! Canonical attribution for every hook-emitted event:
//!
//! - `source = Source::Hook`
//! - `trigger = Trigger::GitHook(<subcommand>)` — the `knotch hook <name>` wrapper.
//! - `session` — set when `session_id` parses as a UUID; otherwise omitted.
//! - `agent_id` — populated when the hook payload carries one (subagent events); `None`
//!   for main-session hook invocations.
//!
//! Model attribution lives on dedicated `ModelSwitched` events,
//! not on every causation. Consumers read the effective
//! `model_timeline` to correlate events with the model active at
//! their timestamp.

use compact_str::CompactString;
use knotch_kernel::{
    Causation,
    causation::{AgentId, SessionId, Source, Trigger},
};

use crate::input::HookInput;

/// Build a `Causation` for a hook-emitted event.
///
/// `agent_id` is populated whenever Claude Code surfaces one —
/// the envelope carries it on every hook that runs inside a
/// subagent scope ([`HookInput::agent_id`]), and `SubagentStop`
/// duplicates it in its variant payload. Main-session hooks
/// leave it `None`; the session id alone identifies the
/// conversation.
#[must_use]
pub fn hook_causation(input: &HookInput, subcommand: &str) -> Causation {
    let mut causation =
        Causation::new(Source::Hook, Trigger::GitHook { name: CompactString::from(subcommand) })
            .with_session(SessionId::parse(input.session_id.as_str()));
    if let Some(agent_id) = input.agent_id() {
        causation = causation.with_agent_id(AgentId::from(agent_id));
    }
    causation
}
