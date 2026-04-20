use knotch_derive::PhaseKind;
use serde::{Deserialize, Serialize};

// `#[derive(PhaseKind)]` requires an enum with only unit variants —
// tuple variants must be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind)]
pub enum P {
    A,
    B(u32),
}

fn main() {}
