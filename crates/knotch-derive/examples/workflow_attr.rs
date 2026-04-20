//! Exercise `#[workflow]` — compiles, runs `WorkflowKind::required_phases`.

#![allow(missing_docs)]

use std::borrow::Cow;

use knotch_derive::{GateKind, PhaseKind, workflow};
use knotch_kernel::{Scope, WorkflowKind};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
pub enum FlowPhase {
    Draft,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlowTask(pub String);
impl knotch_kernel::MilestoneKind for FlowTask {
    fn id(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
pub enum FlowGate {
    Approve,
}

fn phases(_: &Scope) -> &'static [FlowPhase] {
    &[FlowPhase::Draft, FlowPhase::Review]
}

#[derive(Debug, Clone, Copy, Default)]
#[workflow(
    name = "attr-flow",
    schema_version = 1,
    phase = FlowPhase,
    milestone = FlowTask,
    gate = FlowGate,
    required_phases = phases,
)]
pub struct AttrFlow;

fn main() {
    assert_eq!(AttrFlow.name(), "attr-flow");
    assert_eq!(AttrFlow.schema_version(), 1);
    assert_eq!(AttrFlow.required_phases(&Scope::Standard).len(), 2);
}
