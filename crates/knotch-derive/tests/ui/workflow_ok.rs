use std::borrow::Cow;

use knotch_derive::{GateKind, PhaseKind, workflow};
use knotch_kernel::{Scope, WorkflowKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind)]
pub enum P { A, B }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct M(pub String);
impl knotch_kernel::MilestoneKind for M {
    fn id(&self) -> Cow<'_, str> { Cow::Borrowed(&self.0) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
pub enum Gt { Ok }

fn phases(_: &Scope) -> &'static [P] { &[P::A, P::B] }

#[derive(Debug, Clone, Copy, Default)]
#[workflow(name = "ok", phase = P, milestone = M, gate = Gt, required_phases = phases)]
pub struct Flow;

fn main() {
    assert_eq!(Flow.name(), "ok");
}
