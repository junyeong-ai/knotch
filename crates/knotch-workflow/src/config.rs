//! `ConfigWorkflow` — a runtime-configured [`WorkflowKind`]
//! implementation whose shape is loaded from TOML rather than
//! compiled into a typed enum.
//!
//! This is the **zero-Rust adoption path**. An adopter declares their
//! phases / gates / milestones / prerequisites in `knotch.toml` and
//! the shipped `knotch` binary dispatches against it — the adopter
//! never writes Rust.
//!
//! The typed reference implementations (`knotch_workflow::Knotch`,
//! the case-study forks under `examples/`) stay available for
//! Rust-first adopters who want compile-time exhaustiveness. Both
//! paths go through the same kernel invariants (fingerprint dedup,
//! optimistic CAS, terminal immutability, kernel-enforced gate
//! ordering).
//!
//! ## Canonical shape
//!
//! [`ConfigWorkflow::canonical`] returns the canonical `Knotch`
//! shape as a `ConfigWorkflow`. The shipped binary uses this when
//! no `[workflow]` table appears in `knotch.toml`; adopters
//! override by editing the section.
//!
//! ## Example
//!
//! ```no_run
//! use knotch_workflow::config::ConfigWorkflow;
//! let w = ConfigWorkflow::canonical();
//! # let _: ConfigWorkflow = w;
//! ```

use std::{borrow::Cow, collections::HashMap, path::Path, sync::Arc};

use compact_str::CompactString;
use knotch_kernel::{Scope, StatusId, WorkflowKind};
use serde::{Deserialize, Serialize};

use crate::dynamic::{DynamicExtension, DynamicGate, DynamicMilestone, DynamicPhase};

/// Errors surfaced by [`ConfigWorkflow::load`] / [`ConfigWorkflow::from_spec`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// I/O error while reading the TOML file.
    #[error("read config file: {0}")]
    Io(#[from] std::io::Error),

    /// TOML parse error.
    #[error("parse TOML: {0}")]
    Toml(#[from] toml::de::Error),

    /// Validation error: duplicate phase/gate id, dangling prereq,
    /// empty required_phases, etc.
    #[error("invalid workflow spec: {0}")]
    Invalid(String),
}

/// Declarative per-phase metadata. Loaded from TOML; stored in the
/// [`ConfigWorkflow`] for lookup at runtime. Phase order is
/// determined by array position — adopters declare phases top-to-
/// bottom in the order they should occur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseSpec {
    /// Kebab-case phase id — the key used by `knotch mark`.
    pub id: CompactString,
    /// Skip codes this phase accepts. Consulted by
    /// `WorkflowKind::accepts_skip_for` when a `PhaseSkipped` event
    /// is proposed.
    #[serde(default)]
    pub accepts_skips: Vec<CompactString>,
}

/// Declarative per-gate metadata. Prerequisites name earlier gates
/// by id; the kernel enforces the graph at append time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateSpec {
    /// Kebab-case gate id — the key used by `knotch gate`.
    pub id: CompactString,
    /// Prerequisite gate ids that must appear on the log first.
    #[serde(default)]
    pub prerequisites: Vec<CompactString>,
}

/// Required-phase lists keyed by scope. Scope variants that appear
/// in `knotch_kernel::Scope` as of SCHEMA v1: `tiny` / `standard`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScopedPhaseMap {
    /// Phases required under [`Scope::Tiny`].
    #[serde(default)]
    pub tiny: Vec<CompactString>,
    /// Phases required under [`Scope::Standard`] (also the fallback
    /// for every non-tiny scope).
    #[serde(default)]
    pub standard: Vec<CompactString>,
}

/// Top-level TOML shape. Sits under the `[workflow]` table in
/// `knotch.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    /// Adopter-chosen workflow name. Ends up in the log-file header
    /// and the fingerprint salt — changing it creates a new
    /// fingerprint namespace.
    pub name: CompactString,
    /// Wire-format version. Bump on incompatible spec changes.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Phase list in canonical order.
    pub phases: Vec<PhaseSpec>,
    /// Per-scope required-phase lists.
    pub required_phases: ScopedPhaseMap,
    /// Gate list with prerequisite edges.
    #[serde(default)]
    pub gates: Vec<GateSpec>,
    /// Statuses considered terminal for the Phase × Status
    /// cross-invariant.
    #[serde(default)]
    pub terminal_statuses: Vec<CompactString>,
    /// Advisory known-status vocabulary — `knotch transition` warns
    /// when the target is outside this list.
    #[serde(default)]
    pub known_statuses: Vec<CompactString>,
    /// Minimum rationale length for gates / forced transitions.
    /// Defaults to the kernel's `DEFAULT_MIN_RATIONALE_CHARS`.
    #[serde(default)]
    pub min_rationale_chars: Option<usize>,
}

const fn default_schema_version() -> u32 {
    1
}

/// Per-phase metadata looked up by `ConfigWorkflow::accepts_skip_for`.
#[derive(Debug, Clone)]
struct PhaseMeta {
    accepts_skips: Vec<CompactString>,
}

/// Runtime-configured workflow. Cheap to clone (internally an `Arc`
/// of the spec + lookup tables).
#[derive(Debug, Clone)]
pub struct ConfigWorkflow {
    spec: Arc<WorkflowSpec>,
    phase_lookup: Arc<HashMap<CompactString, DynamicPhase>>,
    phase_meta: Arc<HashMap<CompactString, PhaseMeta>>,
    gate_lookup: Arc<HashMap<CompactString, DynamicGate>>,
    gate_prereqs: Arc<HashMap<CompactString, Vec<DynamicGate>>>,
    required_tiny: Arc<Vec<DynamicPhase>>,
    required_standard: Arc<Vec<DynamicPhase>>,
    terminal_statuses: Arc<Vec<CompactString>>,
}

impl Default for ConfigWorkflow {
    fn default() -> Self {
        Self::canonical()
    }
}

impl ConfigWorkflow {
    /// Canonical `Knotch` shape — byte-identical to the
    /// `knotch_workflow::Knotch` typed impl's wire form except
    /// that phases serialise as `{"id": "specify", ...}` rather
    /// than bare strings. Greenfield adopters start here; `knotch
    /// init` writes this to `knotch.toml` for editing.
    #[must_use]
    pub fn canonical() -> Self {
        let spec: WorkflowSpec = toml::from_str(CANONICAL_TOML).expect("canonical.toml parses");
        Self::from_spec(spec).expect("canonical spec validates")
    }

    /// Load a workflow spec from a TOML file.
    ///
    /// The file must contain a top-level `[workflow]` table OR be the
    /// spec directly (when loaded out-of-band, e.g. for testing).
    ///
    /// # Errors
    /// See [`ConfigError`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)?;
        let spec: WorkflowSpec = if raw.contains("[workflow]") {
            #[derive(Deserialize)]
            struct Wrapper {
                workflow: WorkflowSpec,
            }
            let wrapper: Wrapper = toml::from_str(&raw)?;
            wrapper.workflow
        } else {
            toml::from_str(&raw)?
        };
        Self::from_spec(spec)
    }

    /// Build a `ConfigWorkflow` from an in-memory spec. Validates
    /// uniqueness + prereq integrity + required-phase references.
    ///
    /// # Errors
    /// Returns `ConfigError::Invalid` on any validation failure.
    pub fn from_spec(spec: WorkflowSpec) -> Result<Self, ConfigError> {
        let mut phase_lookup: HashMap<CompactString, DynamicPhase> = HashMap::new();
        let mut phase_meta: HashMap<CompactString, PhaseMeta> = HashMap::new();
        for p in &spec.phases {
            if phase_lookup.contains_key(&p.id) {
                return Err(ConfigError::Invalid(format!("duplicate phase id `{}`", p.id)));
            }
            phase_lookup.insert(p.id.clone(), DynamicPhase(p.id.clone()));
            phase_meta.insert(p.id.clone(), PhaseMeta { accepts_skips: p.accepts_skips.clone() });
        }

        let mut gate_lookup: HashMap<CompactString, DynamicGate> = HashMap::new();
        let mut gate_prereqs: HashMap<CompactString, Vec<DynamicGate>> = HashMap::new();
        for g in &spec.gates {
            if gate_lookup.contains_key(&g.id) {
                return Err(ConfigError::Invalid(format!("duplicate gate id `{}`", g.id)));
            }
            gate_lookup.insert(g.id.clone(), DynamicGate(g.id.clone()));
        }
        for g in &spec.gates {
            let mut prereqs = Vec::with_capacity(g.prerequisites.len());
            for pid in &g.prerequisites {
                if !gate_lookup.contains_key(pid) {
                    return Err(ConfigError::Invalid(format!(
                        "gate `{}` prerequisite `{pid}` is not a declared gate",
                        g.id,
                    )));
                }
                prereqs.push(DynamicGate(pid.clone()));
            }
            gate_prereqs.insert(g.id.clone(), prereqs);
        }

        let required_tiny = resolve_required(&spec.required_phases.tiny, &phase_lookup, "tiny")?;
        let required_standard =
            resolve_required(&spec.required_phases.standard, &phase_lookup, "standard")?;
        if required_standard.is_empty() {
            return Err(ConfigError::Invalid(
                "required_phases.standard must list at least one phase id".into(),
            ));
        }

        Ok(Self {
            terminal_statuses: Arc::new(spec.terminal_statuses.clone()),
            spec: Arc::new(spec),
            phase_lookup: Arc::new(phase_lookup),
            phase_meta: Arc::new(phase_meta),
            gate_lookup: Arc::new(gate_lookup),
            gate_prereqs: Arc::new(gate_prereqs),
            required_tiny: Arc::new(required_tiny),
            required_standard: Arc::new(required_standard),
        })
    }

    /// Look up a gate by id. Used by the CLI's `gate` subcommand.
    /// Kernel-level ordering enforcement calls
    /// [`WorkflowKind::prerequisites_for`] which delegates to this
    /// workflow's `gate_prereqs` map.
    #[must_use]
    pub fn gate(&self, id: &str) -> Option<&DynamicGate> {
        self.gate_lookup.get(id)
    }

    /// Borrow the loaded spec — useful for CLI `doctor` / `show` that
    /// want to inspect the underlying declaration.
    #[must_use]
    pub fn spec(&self) -> &WorkflowSpec {
        &self.spec
    }
}

fn resolve_required(
    ids: &[CompactString],
    phases: &HashMap<CompactString, DynamicPhase>,
    scope_name: &str,
) -> Result<Vec<DynamicPhase>, ConfigError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let phase = phases.get(id).ok_or_else(|| {
            ConfigError::Invalid(format!(
                "required_phases.{scope_name} references `{id}` which is not a declared phase",
            ))
        })?;
        out.push(phase.clone());
    }
    Ok(out)
}

impl WorkflowKind for ConfigWorkflow {
    type Phase = DynamicPhase;
    type Milestone = DynamicMilestone;
    type Gate = DynamicGate;
    type Extension = DynamicExtension;

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.spec.name.as_str())
    }

    fn schema_version(&self) -> u32 {
        self.spec.schema_version
    }

    fn required_phases(&self, scope: &Scope) -> Cow<'_, [Self::Phase]> {
        match scope {
            Scope::Tiny => Cow::Borrowed(self.required_tiny.as_slice()),
            _ => Cow::Borrowed(self.required_standard.as_slice()),
        }
    }

    fn is_terminal_status(&self, status: &StatusId) -> bool {
        self.terminal_statuses.iter().any(|t| t.as_str() == status.as_str())
    }

    fn parse_phase(&self, text: &str) -> Option<Self::Phase> {
        self.phase_lookup.get(text).cloned()
    }

    fn parse_gate(&self, text: &str) -> Option<Self::Gate> {
        self.gate_lookup.get(text).cloned()
    }

    fn parse_milestone(&self, text: &str) -> Option<Self::Milestone> {
        Some(DynamicMilestone(CompactString::from(text)))
    }

    fn known_statuses(&self) -> Vec<Cow<'_, str>> {
        self.spec.known_statuses.iter().map(|s| Cow::Borrowed(s.as_str())).collect()
    }

    fn min_rationale_chars(&self) -> usize {
        self.spec
            .min_rationale_chars
            .unwrap_or(knotch_kernel::rationale::DEFAULT_MIN_RATIONALE_CHARS)
    }

    fn prerequisites_for<'a>(&'a self, gate: &'a Self::Gate) -> Cow<'a, [Self::Gate]> {
        self.gate_prereqs
            .get(gate.0.as_str())
            .map(|v| Cow::Borrowed(v.as_slice()))
            .unwrap_or(Cow::Borrowed(&[]))
    }

    fn accepts_skip_for(
        &self,
        phase: &Self::Phase,
        reason: &knotch_kernel::event::SkipKind,
    ) -> bool {
        use knotch_kernel::event::SkipKind;
        let Some(meta) = self.phase_meta.get(phase.0.as_str()) else {
            return false;
        };
        match reason {
            SkipKind::ScopeTooNarrow => meta.accepts_skips.iter().any(|c| c == "scope"),
            SkipKind::Amnesty { code } | SkipKind::Custom { code } => {
                meta.accepts_skips.iter().any(|c| c == code.as_str() || c == "*")
            }
            _ => false,
        }
    }
}

/// Canonical knotch workflow as TOML. Shipped verbatim to
/// `knotch.toml` by `knotch init`; loaded by
/// [`ConfigWorkflow::canonical`] at startup when no project-local
/// override is present.
pub const CANONICAL_TOML: &str = include_str!("../canonical.toml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_parses_and_validates() {
        let w = ConfigWorkflow::canonical();
        assert_eq!(w.name(), "knotch");
        assert_eq!(w.schema_version(), 1);
        assert_eq!(w.required_phases(&Scope::Standard).len(), 5);
        assert_eq!(w.required_phases(&Scope::Tiny).len(), 3);
    }

    #[test]
    fn canonical_terminal_statuses_match_typed_knotch() {
        let w = ConfigWorkflow::canonical();
        for s in ["archived", "abandoned", "superseded", "deprecated"] {
            assert!(w.is_terminal_status(&StatusId::new(s)));
        }
        assert!(!w.is_terminal_status(&StatusId::new("draft")));
    }

    #[test]
    fn canonical_known_statuses_includes_every_terminal() {
        let w = ConfigWorkflow::canonical();
        let known = w.known_statuses();
        for t in ["archived", "abandoned", "superseded", "deprecated"] {
            assert!(known.iter().any(|s| s.as_ref() == t), "missing terminal `{t}`",);
        }
    }

    #[test]
    fn canonical_gate_prereqs_form_a_ladder() {
        let w = ConfigWorkflow::canonical();
        let g0 = w.gate("g0-scope").unwrap();
        assert!(w.prerequisites_for(g0).is_empty());
        let g1 = w.gate("g1-clarify").unwrap();
        assert_eq!(w.prerequisites_for(g1).len(), 1);
        let g4 = w.gate("g4-drift").unwrap();
        assert_eq!(w.prerequisites_for(g4).len(), 4);
    }

    #[test]
    fn from_spec_rejects_duplicate_phase_ids() {
        let spec = WorkflowSpec {
            name: "dup".into(),
            schema_version: 1,
            phases: vec![
                PhaseSpec { id: "a".into(), accepts_skips: vec![] },
                PhaseSpec { id: "a".into(), accepts_skips: vec![] },
            ],
            required_phases: ScopedPhaseMap { tiny: vec!["a".into()], standard: vec!["a".into()] },
            gates: vec![],
            terminal_statuses: vec![],
            known_statuses: vec![],
            min_rationale_chars: None,
        };
        let err = ConfigWorkflow::from_spec(spec).expect_err("duplicate must fail");
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn from_spec_rejects_dangling_gate_prereq() {
        let spec = WorkflowSpec {
            name: "ghost".into(),
            schema_version: 1,
            phases: vec![PhaseSpec { id: "a".into(), accepts_skips: vec![] }],
            required_phases: ScopedPhaseMap { tiny: vec!["a".into()], standard: vec!["a".into()] },
            gates: vec![GateSpec { id: "g1".into(), prerequisites: vec!["g0".into()] }],
            terminal_statuses: vec![],
            known_statuses: vec![],
            min_rationale_chars: None,
        };
        let err = ConfigWorkflow::from_spec(spec).expect_err("dangling prereq must fail");
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn from_spec_rejects_empty_standard_required() {
        let spec = WorkflowSpec {
            name: "empty".into(),
            schema_version: 1,
            phases: vec![PhaseSpec { id: "a".into(), accepts_skips: vec![] }],
            required_phases: ScopedPhaseMap { tiny: vec![], standard: vec![] },
            gates: vec![],
            terminal_statuses: vec![],
            known_statuses: vec![],
            min_rationale_chars: None,
        };
        let err = ConfigWorkflow::from_spec(spec).expect_err("empty standard must fail");
        assert!(matches!(err, ConfigError::Invalid(_)));
    }
}
