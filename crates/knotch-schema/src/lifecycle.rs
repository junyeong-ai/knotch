//! Lifecycle FSM — enforces valid `StatusId` transitions and the
//! Phase × Status cross-invariant (see `.claude/rules/preconditions.md`).

use std::collections::HashSet;

use compact_str::CompactString;
use knotch_kernel::{Rationale, StatusId};

/// A request to transition a unit to a new status. Built by callers
/// and evaluated by [`LifecycleFsm::evaluate`].
#[derive(Debug, Clone)]
pub struct TransitionRequest {
    /// Current status, or `None` for a freshly-created unit.
    pub current: Option<StatusId>,
    /// Desired new status.
    pub target: StatusId,
    /// Bypass the cross-invariant. `forced = true` requires
    /// [`TransitionRequest::rationale`] to be present.
    pub forced: bool,
    /// Non-empty rationale on forced transitions.
    pub rationale: Option<Rationale>,
    /// Have every required phase been resolved (completed or skipped)?
    pub all_phases_resolved: bool,
}

/// Status FSM.
#[derive(Debug, Clone, Default)]
pub struct LifecycleFsm {
    terminal: HashSet<CompactString>,
}

impl LifecycleFsm {
    /// Build an FSM builder.
    #[must_use]
    pub fn builder() -> Self {
        Self::default()
    }

    /// Mark `status` as terminal (entering it requires every required
    /// phase to be resolved unless the transition is forced).
    #[must_use]
    pub fn terminal(mut self, status: impl Into<CompactString>) -> Self {
        self.terminal.insert(status.into());
        self
    }

    /// Is the supplied status terminal?
    #[must_use]
    pub fn is_terminal(&self, status: &StatusId) -> bool {
        self.terminal.iter().any(|t| t.as_str() == status.as_str())
    }

    /// Evaluate a transition request; returns `Ok(())` when the
    /// transition is legal.
    ///
    /// # Errors
    /// Returns [`LifecycleError`] describing the first invariant
    /// violated.
    pub fn evaluate(&self, request: &TransitionRequest) -> Result<(), LifecycleError> {
        if let Some(current) = &request.current {
            if current.as_str() == request.target.as_str() {
                return Err(LifecycleError::NoOpTransition { status: request.target.clone() });
            }
        }

        if request.forced && request.rationale.is_none() {
            return Err(LifecycleError::ForcedWithoutRationale);
        }

        if self.is_terminal(&request.target) && !request.forced && !request.all_phases_resolved {
            return Err(LifecycleError::RequiredPhasesPending { target: request.target.clone() });
        }

        Ok(())
    }
}

/// Lifecycle-FSM error taxonomy.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum LifecycleError {
    /// The request asks to transition to the current status.
    #[error("no-op transition to {status}")]
    NoOpTransition {
        /// Current status, unchanged by the no-op.
        status: StatusId,
    },
    /// Forced transition without a rationale.
    #[error("forced status transition requires a rationale")]
    ForcedWithoutRationale,
    /// Terminal status attempted while required phases remain.
    #[error("cannot transition to {target} — required phases remain unresolved")]
    RequiredPhasesPending {
        /// Target terminal status.
        target: StatusId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fsm() -> LifecycleFsm {
        LifecycleFsm::builder().terminal("archived").terminal("abandoned")
    }

    #[test]
    fn rejects_noop() {
        let err = fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("draft")),
                target: StatusId::new("draft"),
                forced: false,
                rationale: None,
                all_phases_resolved: true,
            })
            .unwrap_err();
        assert!(matches!(err, LifecycleError::NoOpTransition { .. }));
    }

    #[test]
    fn rejects_forced_without_rationale() {
        let err = fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("draft")),
                target: StatusId::new("archived"),
                forced: true,
                rationale: None,
                all_phases_resolved: false,
            })
            .unwrap_err();
        assert_eq!(err, LifecycleError::ForcedWithoutRationale);
    }

    #[test]
    fn rejects_terminal_with_pending_phases() {
        let err = fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("planning")),
                target: StatusId::new("archived"),
                forced: false,
                rationale: None,
                all_phases_resolved: false,
            })
            .unwrap_err();
        assert!(matches!(err, LifecycleError::RequiredPhasesPending { .. }));
    }

    #[test]
    fn accepts_terminal_with_all_phases_resolved() {
        fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("in_review")),
                target: StatusId::new("archived"),
                forced: false,
                rationale: None,
                all_phases_resolved: true,
            })
            .expect("legal");
    }

    #[test]
    fn accepts_forced_with_rationale() {
        fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("planning")),
                target: StatusId::new("abandoned"),
                forced: true,
                rationale: Some(Rationale::new("deprioritized by product").expect("r")),
                all_phases_resolved: false,
            })
            .expect("legal");
    }

    #[test]
    fn accepts_non_terminal_transition() {
        fsm()
            .evaluate(&TransitionRequest {
                current: Some(StatusId::new("draft")),
                target: StatusId::new("planning"),
                forced: false,
                rationale: None,
                all_phases_resolved: false,
            })
            .expect("legal");
    }
}
