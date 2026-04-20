//! Hook decision output.
//!
//! Every hook function returns exactly one [`HookOutput`]. The
//! binary entry point (for example `knotch-cli`) maps each variant
//! to the Claude Code hook wire format:
//!
//! | Variant             | stdout                                                             | exit |
//! |---------------------|--------------------------------------------------------------------|------|
//! | `Continue`          | empty                                                              | 0    |
//! | `Context(ctx)`      | JSON with `hookSpecificOutput.additionalContext = ctx`             | 0    |
//! | `Block{reason}`     | empty (reason on stderr)                                           | 2    |
//! | `UpdateInput(v)`    | JSON with `permissionDecision: "allow" + updatedInput = v`         | 0    |
//! | `Ask{reason}`       | JSON with `permissionDecision: "ask" + permissionDecisionReason`   | 0    |
//!
//! Exit codes 1 / 3+ are never emitted — Claude Code treats them as
//! non-blocking, which would let a tool call proceed with no ledger
//! record.
//!
//! ## Variant applicability
//!
//! `UpdateInput` and `Ask` are only meaningful on `PreToolUse`.
//! Returning them from other events produces a no-op JSON payload
//! (Claude Code ignores unknown `permissionDecision` outside
//! `PreToolUse`). `knotch-agent` does not statically enforce this —
//! the hook author picks an output valid for their event.

/// Decision returned from every `knotch-agent::*` hook entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutput {
    /// Let the action proceed, nothing extra to say.
    Continue,
    /// Let the action proceed; inject `context` into the
    /// conversation. Applicable to `SessionStart`,
    /// `UserPromptSubmit`, `PostToolUse`, `SubagentStop`.
    Context(String),
    /// Cancel the action. Stderr receives `reason`; the binary
    /// wrapper exits 2.
    Block {
        /// Human-readable explanation shown to the agent.
        reason: String,
    },
    /// `PreToolUse` only — allow the tool call but rewrite its
    /// input payload. Typical use: inject a `Knotch-Milestone:` git
    /// trailer into a commit command before it runs.
    UpdateInput(serde_json::Value),
    /// `PreToolUse` only — escalate to the user for approval with
    /// `reason` shown in the confirmation dialog.
    Ask {
        /// Human-readable rationale shown in the permission prompt.
        reason: String,
    },
}

impl HookOutput {
    /// Helper: wrap a reason into a [`HookOutput::Block`].
    #[must_use]
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block { reason: reason.into() }
    }

    /// Helper: wrap a context string into a [`HookOutput::Context`].
    #[must_use]
    pub fn context(ctx: impl Into<String>) -> Self {
        Self::Context(ctx.into())
    }

    /// Helper: wrap an updated tool input.
    #[must_use]
    pub fn update_input(v: serde_json::Value) -> Self {
        Self::UpdateInput(v)
    }

    /// Helper: wrap an ask-for-permission reason.
    #[must_use]
    pub fn ask(reason: impl Into<String>) -> Self {
        Self::Ask { reason: reason.into() }
    }
}
