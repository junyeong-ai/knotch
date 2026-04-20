// Examples are tutorial code, not a published library surface.
#![allow(missing_docs)]

//! Compliance workflow with a typed `Extension` payload.
//!
//! Most examples use `Extension = ()`. This one demonstrates how to
//! attach structured audit metadata (reviewer id + timestamp) to
//! every event so compliance queries don't need a sidecar database.
//!
//! - Phases: `Submitted` → `Reviewed` → `Approved` → `Implemented` → `Audited`
//! - Milestone: `ChangeId`
//! - Gates: `SecurityReview`, `ComplianceReview`, `ApprovalBoard`
//! - Extension: [`AuditMeta`] carrying `reviewer` + `stamp`
//! - Terminal statuses: `approved_closed`, `rejected_closed`

use compact_str::CompactString;
use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{
    AppendMode, Causation, ExtensionKind, Proposal, Rationale, Repository, Scope, StatusId, UnitId,
    WorkflowKind, event::EventBody, status::Decision,
};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum CompliancePhase {
    Submitted,
    Reviewed,
    Approved,
    Implemented,
    Audited,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct ChangeId(pub CompactString);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceGate {
    SecurityReview,
    ComplianceReview,
    ApprovalBoard,
}

/// Typed audit metadata attached to every event. Compliance queries
/// read these directly off the event stream — no sidecar DB needed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditMeta {
    /// Reviewer / actor id (opaque).
    #[serde(default)]
    pub reviewer: Option<CompactString>,
    /// ISO-8601 timestamp recorded by the auditor.
    #[serde(default)]
    pub stamp: Option<CompactString>,
}

impl ExtensionKind for AuditMeta {}

#[derive(Debug, Clone, Copy, Default)]
pub struct Compliance;

const PHASES: [CompliancePhase; 5] = [
    CompliancePhase::Submitted,
    CompliancePhase::Reviewed,
    CompliancePhase::Approved,
    CompliancePhase::Implemented,
    CompliancePhase::Audited,
];

impl WorkflowKind for Compliance {
    type Phase = CompliancePhase;
    type Milestone = ChangeId;
    type Gate = ComplianceGate;
    type Extension = AuditMeta;

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("compliance")
    }
    fn schema_version(&self) -> u32 {
        1
    }

    fn required_phases(&self, _scope: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }

    fn is_terminal_status(&self, status: &StatusId) -> bool {
        matches!(status.as_str(), "approved_closed" | "rejected_closed" | "abandoned")
    }

    fn min_rationale_chars(&self) -> usize {
        // Compliance requires substantive rationales.
        32
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = FileRepository::<Compliance>::new(dir.path(), Compliance);
    let unit = UnitId::try_new("change-2026-04-19-001").unwrap();

    let now = || CompactString::from("2026-04-19T10:00:00Z");

    append(
        &repo,
        &unit,
        AuditMeta { reviewer: Some("system".into()), stamp: Some(now()) },
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await?;
    append(
        &repo,
        &unit,
        AuditMeta { reviewer: Some("alice@corp".into()), stamp: Some(now()) },
        EventBody::GateRecorded {
            gate: ComplianceGate::SecurityReview,
            decision: Decision::Approved,
            rationale: Rationale::new(
                "threat-model reviewed: no new attack surface introduced by this change",
            )
            .unwrap(),
        },
    )
    .await?;
    append(
        &repo,
        &unit,
        AuditMeta { reviewer: Some("bob@corp".into()), stamp: Some(now()) },
        EventBody::GateRecorded {
            gate: ComplianceGate::ComplianceReview,
            decision: Decision::Approved,
            rationale: Rationale::new(
                "SOC2 §CC7.1 impact assessed — logging and alerting paths unchanged",
            )
            .unwrap(),
        },
    )
    .await?;
    append(
        &repo,
        &unit,
        AuditMeta { reviewer: Some("board".into()), stamp: Some(now()) },
        EventBody::StatusTransitioned {
            target: StatusId::new("approved_closed"),
            forced: false,
            rationale: Some(
                Rationale::new(
                    "approved by CAB on 2026-04-19 after security + compliance sign-off",
                )
                .unwrap(),
            ),
        },
    )
    .await?;

    let log = repo.load(&unit).await?;
    println!("change:         {}", unit.as_str());
    println!("status:         {:?}", knotch_kernel::project::current_status(&log));
    println!("events:         {}", log.events().len());
    // Demonstrate that extension data is readable off the stream.
    let reviewers: Vec<_> =
        log.events().iter().filter_map(|evt| evt.extension.reviewer.as_ref()).collect();
    println!("reviewers seen: {:?}", reviewers);
    Ok(())
}

async fn append<R>(
    repo: &R,
    unit: &UnitId,
    extension: AuditMeta,
    body: EventBody<Compliance>,
) -> anyhow::Result<()>
where
    R: Repository<Compliance>,
{
    let proposal = Proposal {
        causation: Causation::cli("example-compliance"),
        extension,
        body,
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    Ok(())
}
