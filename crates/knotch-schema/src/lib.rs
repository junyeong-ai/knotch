//! Frontmatter schema builder and lifecycle FSM.
//!
//! Two concerns, one crate:
//!
//! - [`FrontmatterSchema`] — declarative validation for per-unit
//!   `spec.md`-style frontmatter. Preset crates declare which fields
//!   are required, which are enumerated, and which must match a
//!   regex; the schema validates an arbitrary TOML/JSON object.
//! - [`LifecycleFsm`] — the Status FSM that accompanies a workflow.
//!   Encodes which `StatusId` values are terminal and enforces the
//!   Phase × Status cross-invariant (see `.claude/rules/preconditions.md`).

pub mod frontmatter;
pub mod lifecycle;

pub use self::{
    frontmatter::{FieldSchema, FieldType, FrontmatterSchema, SchemaError},
    lifecycle::{LifecycleError, LifecycleFsm, TransitionRequest},
};
