//! Spec-driven workflow example with G0-G6 checkpoint gates.
#![allow(missing_docs)]

use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{Scope, WorkflowKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind)]
#[serde(rename_all = "snake_case")]
pub enum SpecPhase { Specify, Design, Implement, Review, Wrapup }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
pub enum Story { UserSignup, PaymentFlow, AuditLog }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum SpecGate { G0Scope, G1Clarify, G2Constitution, G3Analyze, G5Review, G6Drift }

#[derive(Debug, Clone, Copy)]
pub struct SpecDriven;

const PHASES_STANDARD: [SpecPhase; 5] = [SpecPhase::Specify, SpecPhase::Design, SpecPhase::Implement, SpecPhase::Review, SpecPhase::Wrapup];
const PHASES_TINY: [SpecPhase; 4] = [SpecPhase::Specify, SpecPhase::Design, SpecPhase::Implement, SpecPhase::Wrapup];

impl WorkflowKind for SpecDriven {
    type Phase = SpecPhase;
    type Milestone = Story;
    type Gate = SpecGate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("specdriven") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, scope: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        match scope {
            Scope::Tiny => std::borrow::Cow::Borrowed(&PHASES_TINY),
            _ => std::borrow::Cow::Borrowed(&PHASES_STANDARD),
        }
    }
}

fn main() {
    use knotch_kernel::PhaseKind as _;
    assert_eq!(SpecPhase::Specify.id(), "specify");
    assert_eq!(SpecPhase::Design.id(), "design");
    assert_eq!(SpecPhase::Wrapup.id(), "wrapup");
    println!("specdriven workflow: {} phases", SpecDriven.required_phases(&Scope::Standard).len());
}
