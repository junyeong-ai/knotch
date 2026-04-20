//! Kernel error taxonomy.

use std::fmt;

use crate::{fingerprint::Fingerprint, time::Timestamp};

/// Boxed adapter error. Adapters (`knotch-storage`, `-lock`, etc.)
/// surface concrete errors and box them into this type at the
/// Repository boundary — giving kernel callers a single error enum
/// to match against.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Errors returned by `Repository` implementations.
///
/// Adapters contribute `Storage` / `Lock` variants by boxing their
/// native error. Kernel-level variants (`Precondition`, `Duplicate`,
/// `SchemaMismatch`, `NonMonotonic`, `Corrupted`) are structured.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RepositoryError {
    /// A proposal's precondition was not satisfied.
    #[error("precondition violated")]
    Precondition(#[source] PreconditionError),
    /// The proposal is a replay — its fingerprint matches an existing
    /// event and the Repository rejected it as a no-op.
    #[error("fingerprint duplicate — proposal replays an existing event")]
    Duplicate(Fingerprint),
    /// Storage-adapter failure.
    #[error("storage backend failure")]
    Storage(#[source] BoxError),
    /// Lock-adapter failure.
    #[error("lock backend failure")]
    Lock(#[source] BoxError),
    /// Serialization / wire-format failure.
    #[error("codec failure")]
    Codec(#[source] serde_json::Error),
    /// The on-disk schema version differs from the compiled version.
    #[error("schema version {found} not supported (expected {expected})")]
    SchemaMismatch {
        /// Version read from the log header.
        found: u32,
        /// Version this build requires.
        expected: u32,
    },
    /// The event log contains a line that does not parse.
    #[error("log corrupted at line {line}")]
    Corrupted {
        /// 1-indexed line number of the first corrupted line.
        line: u64,
    },
    /// An incoming proposal's timestamp is earlier than the log's last
    /// event timestamp.
    #[error("append ordering violated — attempted at={attempted} < last={last}")]
    NonMonotonic {
        /// Timestamp on the proposal.
        attempted: Timestamp,
        /// Timestamp on the last log entry.
        last: Timestamp,
    },
    /// The log header carries a `fingerprint_salt` that does not match
    /// `W::fingerprint_salt()` on the current build. Either the
    /// workflow impl changed its salt without bumping
    /// `SCHEMA_VERSION` (see `.claude/rules/fingerprint.md`) or the
    /// log belongs to a different workflow at the same storage root.
    #[error(
        "fingerprint_salt mismatch — stored={stored:?} current={current:?} (bump SCHEMA_VERSION \
         and ship a SchemaMigrator if the salt legitimately changed)"
    )]
    SaltMismatch {
        /// Base64 salt stored in the log header.
        stored: String,
        /// Base64 salt computed from `W::fingerprint_salt()`.
        current: String,
    },
}

/// Precondition-level error taxonomy. Projected into
/// `RepositoryError::Precondition` at the Repository boundary.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreconditionError {
    /// Attempted to create a unit that already exists.
    #[error("unit already created")]
    AlreadyCreated,
    /// Attempted to complete a phase that is already complete.
    #[error("phase {0} already completed")]
    PhaseAlreadyCompleted(String),
    /// A declared artifact path does not exist on the filesystem at
    /// append time.
    #[error("required artifact not found at {path:?}")]
    ArtifactMissing {
        /// Offending path (as declared in `ArtifactList`).
        path: String,
    },
    /// Attempted to skip a phase that refused the supplied reason.
    #[error("phase {phase} refused skip reason {reason}")]
    SkipRejected {
        /// Phase name.
        phase: String,
        /// Stringified skip reason.
        reason: String,
    },
    /// A commit referenced by the proposal could not be verified.
    #[error("commit {0} is not verifiable")]
    CommitUnverifiable(String),
    /// Commit kind is not an implementation kind — docs/chore/test/
    /// ci/build/style/revert cannot ship a milestone.
    #[error("commit kind {kind:?} cannot ship a milestone")]
    CommitKindNotImplementation {
        /// The rejected kind.
        kind: String,
    },
    /// The caller claimed a better status than the VCS observed.
    #[error("status downgrade: claimed {claimed}, observed {observed}")]
    StatusDowngrade {
        /// Status carried on the proposal.
        claimed: String,
        /// Status returned by VCS::verify.
        observed: String,
    },
    /// A milestone is already in the effective shipped set; shipping
    /// it again requires a revert first.
    #[error("milestone {0} is already shipped")]
    MilestoneAlreadyShipped(String),
    /// Attempted to revert a milestone that is not currently shipped.
    #[error("milestone {0} is not in the shipped set")]
    MilestoneNotShipped(String),
    /// `MilestoneVerified` was proposed but no prior `MilestoneShipped`
    /// with `CommitStatus::Pending` matches.
    #[error("no pending ship for milestone {milestone} at commit {commit}")]
    NoPendingShip {
        /// Milestone id.
        milestone: String,
        /// Commit id.
        commit: String,
    },
    /// A rationale shorter than the configured minimum was supplied.
    #[error("rationale shorter than {min} chars (got {actual})")]
    RationaleTooShort {
        /// Required minimum.
        min: usize,
        /// Actual length.
        actual: usize,
    },
    /// `StatusTransitioned` to the current status is a no-op.
    #[error("no-op status transition to {0}")]
    NoOpStatusTransition(String),
    /// A retry proposal's attempt counter is not strictly greater
    /// than the prior maximum for the same anchor.
    #[error("attempt {attempt} not greater than prior max {prior}")]
    NonMonotonicAttempt {
        /// Proposed attempt.
        attempt: u32,
        /// Prior maximum.
        prior: u32,
    },
    /// `ReconcileRecovered` proposed without a preceding `ReconcileFailed`.
    #[error("no prior reconcile failure to recover from")]
    NoPriorFailure,
    /// Attempted to supersede an event that is already superseded.
    #[error("event {0} is already superseded")]
    AlreadySuperseded(String),
    /// Attempted to supersede an event id that is absent from the log.
    #[error("supersede target {0} not found in log")]
    SupersedeTargetMissing(String),
    /// Forced status transition without a supplied rationale.
    #[error("forced status transition requires a rationale")]
    ForcedWithoutRationale,
    /// Non-forced terminal status transition while required phases
    /// remain unresolved (Phase × Status cross-invariant).
    #[error("required phase {phase} not resolved before terminal status")]
    RequiredPhaseNotResolved {
        /// Phase that prevented the transition.
        phase: String,
    },
    /// Attempt to append a non-`EventSuperseded` variant against a
    /// unit whose current status is terminal
    /// (per `W::is_terminal_status`). Terminal units are immutable
    /// except via supersede; use `knotch supersede <event-id>` to
    /// undo the transition first.
    #[error(
        "cannot append against unit in terminal status `{status}` — use EventSuperseded to roll back first"
    )]
    AppendAgainstTerminalUnit {
        /// The terminal status identifier reached by the unit.
        status: String,
    },
    /// Extension-contributed precondition failed.
    #[error("extension precondition rejected: {0}")]
    Extension(String),
    /// A `GateRecorded` proposal's prerequisite gate is absent from
    /// the log. Record every prerequisite declared by
    /// [`GateKind::prerequisites`](crate::workflow::GateKind::prerequisites)
    /// before advancing to `gate`.
    #[error("gate {gate} requires prior {missing} — record it first")]
    GateOutOfOrder {
        /// The gate the caller tried to record.
        gate: String,
        /// The first missing prerequisite (there may be more).
        missing: String,
    },
}

impl fmt::Display for crate::status::StatusId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
