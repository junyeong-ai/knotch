#![allow(missing_docs)]

//! # P1-5 example: atomic multi-proposal append
//!
//! `AppendMode::AllOrNothing` makes a multi-proposal batch commit
//! together or reject the whole batch. Use it when two events must
//! land as a single transaction — here, recording a gate decision
//! and transitioning status in one commit so no observer can ever
//! see the gate without the transition.
//!
//! Run with `cargo run -p knotch-example-batch-append`.

use knotch_kernel::{
    AppendMode, Causation, Decision, Proposal, Rationale, Repository, Scope, StatusId, UnitId,
    causation::{Principal, Source, Trigger},
    event::EventBody,
};
use knotch_workflow::KnotchGate;

fn causation() -> Causation {
    Causation::new(
        Source::Cli,
        Principal::System { service: "batch-append-example".into() },
        Trigger::Command { name: "test".into() },
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = tempfile::tempdir()?;
    let repo = knotch_workflow::build_repository(state.path());
    let unit = UnitId::try_new("batch-demo").unwrap();

    // Seed the unit.
    repo.append(
        &unit,
        vec![Proposal {
            causation: causation(),
            extension: (),
            body: EventBody::UnitCreated { scope: Scope::Standard },
            supersedes: None,
        }],
        AppendMode::BestEffort,
    )
    .await?;

    // Two proposals — gate approval + status transition — must land
    // atomically. If either precondition fails, neither is kept.
    let batch = vec![
        Proposal {
            causation: causation(),
            extension: (),
            body: EventBody::GateRecorded {
                gate: KnotchGate::G3Review,
                decision: Decision::Approved,
                rationale: Rationale::new("review pass clean — shipping").unwrap(),
            },
            supersedes: None,
        },
        Proposal {
            causation: causation(),
            extension: (),
            body: EventBody::StatusTransitioned {
                target: StatusId::new("shipped"),
                forced: false,
                rationale: None,
            },
            supersedes: None,
        },
    ];

    let report = repo.append(&unit, batch, AppendMode::AllOrNothing).await?;
    println!(
        "atomic batch landed: accepted={} rejected={}",
        report.accepted.len(),
        report.rejected.len(),
    );
    for evt in &report.accepted {
        println!("  - {}", evt.body.kind_tag());
    }
    Ok(())
}
