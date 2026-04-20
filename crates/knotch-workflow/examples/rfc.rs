//! RFC workflow example.
#![allow(missing_docs)]

use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{Scope, WorkflowKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind)]
#[serde(rename_all = "snake_case")]
pub enum RfcPhase { Draft, Discuss, Ratified }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
pub enum RfcMilestone { SignOffSlack, MergeMain }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum RfcGate { Consensus, Lgtm }

#[derive(Debug, Clone, Copy)]
pub struct Rfc;

const PHASES: [RfcPhase; 3] = [RfcPhase::Draft, RfcPhase::Discuss, RfcPhase::Ratified];

impl WorkflowKind for Rfc {
    type Phase = RfcPhase;
    type Milestone = RfcMilestone;
    type Gate = RfcGate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("rfc") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

fn main() {
    use knotch_kernel::PhaseKind as _;
    assert_eq!(RfcPhase::Draft.id(), "draft");
    assert_eq!(RfcPhase::Discuss.id(), "discuss");
    assert_eq!(RfcPhase::Ratified.id(), "ratified");
    println!("rfc workflow: {} phases", Rfc.required_phases(&Scope::Standard).len());
}
