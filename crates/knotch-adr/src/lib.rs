//! Specialised knotch workflow for Architectural Decision Record
//! (ADR) lifecycles.
//!
//! Each ADR is modelled as one unit; the workflow carries it
//! through `proposed → active → {superseded, deprecated}`. The
//! canonical `Knotch` workflow doesn't fit — ADRs don't have
//! phases in the development sense — so `knotch-adr` ships its own
//! `WorkflowKind` impl that composes:
//!
//! - A [`FrontmatterSchema`] requiring `id`, `title`, `status`,
//!   `created` in the ADR markdown file.
//! - A [`LifecycleFsm`] encoding the four-state transition graph.
//!
//! The crate intentionally refuses to pick a numbering scheme
//! (`NNNN-slug` vs free-form), a section structure (Status /
//! Context / Decision / Consequences vs anything else), or a
//! promotion pipeline. Those are adopter-specific. Composing
//! `knotch-adr::Adr` with `knotch-frontmatter::sync_status_on_file`
//! in the adopter's hook / skill gives the full picture.
//!
//! ```no_run
//! use knotch_adr::{Adr, frontmatter_schema, lifecycle_fsm};
//! let schema = frontmatter_schema();
//! let fsm = lifecycle_fsm();
//! # let _: knotch_schema::FrontmatterSchema = schema;
//! # let _: knotch_schema::LifecycleFsm = fsm;
//! # let _ = std::marker::PhantomData::<Adr>;
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use compact_str::CompactString;
use knotch_derive::{GateKind, MilestoneKind, PhaseKind};
use knotch_kernel::{Scope, StatusId, WorkflowKind};
use knotch_schema::{FieldSchema, FieldType, FrontmatterSchema, LifecycleFsm};
use knotch_storage::FileRepository;
use serde::{Deserialize, Serialize};

/// The sole ADR "phase" — ADRs don't have a dev-time phase arc,
/// but `WorkflowKind::Phase` must be inhabited, so we ship a
/// single `Decided` phase that completes when the ADR is first
/// authored.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, PhaseKind,
)]
#[serde(rename_all = "snake_case")]
pub enum AdrPhase {
    /// The decision has been captured in writing.
    Decided,
}

/// ADR identifier — free-form slug. Adopters choose the numbering
/// scheme (`NNNN-slug`, `YYYY-MM-DD-slug`, etc.) by picking the
/// string they pass in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, MilestoneKind)]
#[serde(transparent)]
pub struct AdrId(pub CompactString);

/// ADR workflow has no gate ladder — status transitions do the
/// gating themselves. The enum is nominally inhabited by a single
/// `Unused` variant so `WorkflowKind::Gate` has a concrete type;
/// callers never emit `GateRecorded` for ADRs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, GateKind)]
#[serde(rename_all = "snake_case")]
pub enum AdrGate {
    /// Present only to satisfy `WorkflowKind::Gate`. Adopters
    /// never record this — ADR lifecycle flows through
    /// `StatusTransitioned` exclusively.
    #[doc(hidden)]
    Unused,
}

/// ADR workflow marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Adr;

const PHASES: [AdrPhase; 1] = [AdrPhase::Decided];

impl WorkflowKind for Adr {
    type Phase = AdrPhase;
    type Milestone = AdrId;
    type Gate = AdrGate;
    type Extension = ();

    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("adr") }
    fn schema_version(&self) -> u32 { 1 }

    fn required_phases(&self, _: &Scope) -> std::borrow::Cow<'_, [Self::Phase]> {
        std::borrow::Cow::Borrowed(&PHASES)
    }

    /// Terminal statuses. `superseded` and `deprecated` are both
    /// terminal — superseded means a newer ADR replaces this one;
    /// deprecated means the decision no longer applies but nothing
    /// replaces it.
    fn is_terminal_status(&self, status: &StatusId) -> bool {
        matches!(status.as_str(), "superseded" | "deprecated")
    }

    fn known_statuses(&self) -> Vec<std::borrow::Cow<'_, str>> {
        ["proposed", "active", "superseded", "deprecated"]
            .iter()
            .map(|s| std::borrow::Cow::Borrowed(*s))
            .collect()
    }
}

/// Build a file-backed ADR repository at `root`. Adopters typically
/// point `root` at a dedicated `adr-state/` directory so ADR events
/// don't mix with other workflows at the same location.
#[must_use]
pub fn build_repository(root: impl Into<PathBuf>) -> FileRepository<Adr> {
    FileRepository::new(root, Adr)
}

/// Minimal frontmatter schema every ADR markdown file must satisfy.
///
/// Requires `id`, `title`, `status`, `created`. Does **not** lock
/// sections, numbering, `supersedes` / `superseded_by` linkage, or
/// anything else adopter-specific — compose with
/// [`knotch_schema::FrontmatterSchema::field`] to add project
/// conventions.
#[must_use]
pub fn frontmatter_schema() -> FrontmatterSchema {
    FrontmatterSchema::builder()
        .field(FieldSchema::required("id", FieldType::String))
        .field(FieldSchema::required("title", FieldType::String))
        .field(FieldSchema::required(
            "status",
            FieldType::Enum(
                Adr.known_statuses()
                    .iter()
                    .map(|s| CompactString::from(s.as_ref()))
                    .collect(),
            ),
        ))
        .field(FieldSchema::required("created", FieldType::String))
}

/// Lifecycle FSM carrying the ADR terminal set. Use it in your
/// transition skill to cross-check the canonical `W::is_terminal_status`
/// answer when ADRs live alongside other workflow families in the
/// same codebase.
#[must_use]
pub fn lifecycle_fsm() -> LifecycleFsm {
    LifecycleFsm::builder()
        .terminal("superseded")
        .terminal("deprecated")
}

/// The canonical template an adopter can use for a new ADR.
/// Adopters wrap this in their own `knotch adr new <slug>` CLI or
/// `/new-adr` skill — this crate deliberately stays out of the CLI
/// surface, so the single template can ship with the workflow
/// without dragging in `clap`.
pub const TEMPLATE: &str = "---\n\
id: {slug}\n\
title: {title}\n\
status: proposed\n\
created: {today}\n\
---\n\
\n\
# {title}\n\
\n\
## Status\n\
Proposed\n\
\n\
## Context\n\
<!-- What is the issue that we're seeing that is motivating this decision? -->\n\
\n\
## Decision\n\
<!-- What is the change that we're proposing and/or doing? -->\n\
\n\
## Consequences\n\
<!-- What becomes easier or more difficult to do because of this change? -->\n";

#[cfg(test)]
mod tests {
    use super::*;
    use knotch_kernel::PhaseKind as _;

    #[test]
    fn schema_required_fields_include_status_enum() {
        let s = frontmatter_schema();
        let obj = serde_json::json!({
            "id": "0001-sample",
            "title": "Sample",
            "status": "proposed",
            "created": "2026-04-19",
        });
        s.validate(obj.as_object().unwrap()).expect("valid");
    }

    #[test]
    fn schema_rejects_non_canonical_status() {
        let s = frontmatter_schema();
        let obj = serde_json::json!({
            "id": "0001-sample",
            "title": "Sample",
            "status": "in_progress",
            "created": "2026-04-19",
        });
        assert!(s.validate(obj.as_object().unwrap()).is_err());
    }

    #[test]
    fn terminal_set_matches_is_terminal_status() {
        for s in ["superseded", "deprecated"] {
            assert!(Adr.is_terminal_status(&StatusId::new(s)));
        }
        for s in ["proposed", "active"] {
            assert!(!Adr.is_terminal_status(&StatusId::new(s)));
        }
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(Adr.schema_version(), 1);
    }

    #[test]
    fn phase_enum_has_single_variant() {
        let p = AdrPhase::Decided;
        assert_eq!(p.id(), "decided");
    }

    #[test]
    fn template_round_trips_via_format() {
        let rendered = TEMPLATE
            .replace("{slug}", "0042-adr-preset")
            .replace("{title}", "ADR preset")
            .replace("{today}", "2026-04-19");
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("status: proposed"));
        assert!(rendered.contains("# ADR preset"));
    }
}
