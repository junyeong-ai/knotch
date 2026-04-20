//! Vibe-coding (AI-pair-programming) workflow.
#![allow(missing_docs)]

use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{Scope, WorkflowKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind)]
#[serde(rename_all = "snake_case")]
pub enum VibePhase { Intent, Explore, Implement, Verify }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(rename_all = "snake_case")]
pub enum VibeMilestone { DraftReady, TestsGreen, HumanApproved }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum VibeGate { IntentClear, Handoff }

#[derive(Debug, Clone, Copy)]
pub struct Vibe;

const PHASES: [VibePhase; 4] = [
    VibePhase::Intent, VibePhase::Explore, VibePhase::Implement, VibePhase::Verify,
];

impl WorkflowKind for Vibe {
    type Phase = VibePhase;
    type Milestone = VibeMilestone;
    type Gate = VibeGate;
    type Extension = ();
    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("vibe") }
    fn schema_version(&self) -> u32 { 1 }
    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> { std::borrow::Cow::Borrowed(&PHASES) }
}

fn main() {
    use knotch_kernel::PhaseKind as _;
    assert_eq!(VibePhase::Intent.id(), "intent");
    assert_eq!(VibePhase::Implement.id(), "implement");
    println!("vibe workflow: {} phases", Vibe.required_phases(&Scope::Standard).len());
}
