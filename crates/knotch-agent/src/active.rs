//! Active-unit pointer(s).
//!
//! Three resolution layers, highest priority first:
//!
//! 1. **`KNOTCH_UNIT` env var** — explicit override for single-shot invocations and for
//!    shells where a session pointer is not available.
//! 2. **`.knotch/sessions/<session_id>.toml`** — per-session pointer. A Claude Code
//!    session forks from the global pointer at `SessionStart` time; subsequent `knotch
//!    unit use` CLI calls elsewhere do not disturb it.
//! 3. **`.knotch/active.toml`** — project-global pointer. Used by the CLI by default and
//!    as the fallback when no session pointer exists.
//!
//! Schema (both files):
//!
//! ```toml
//! unit = "signup-flow"
//! selected_at = "2026-04-19T10:30:00Z"
//! source = "cli"
//! ```
//!
//! An absent or empty `unit` encodes "no active unit".

use std::path::{Path, PathBuf};

use knotch_kernel::UnitId;
use serde::{Deserialize, Serialize};

use crate::error::HookError;

/// Environment variable that overrides every on-disk pointer.
pub const UNIT_ENV_VAR: &str = "KNOTCH_UNIT";

/// Resolved active-unit state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveUnit {
    /// A pointer resolved to a unit slug.
    Active(UnitId),
    /// `knotch.toml` present but no pointer set.
    Uninitialized,
    /// No `knotch.toml` — not a knotch project.
    NoProject,
}

/// On-disk schema for active-pointer TOML files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActiveToml {
    /// The active unit slug. Empty string encodes "no active unit".
    #[serde(default)]
    pub unit: String,
    /// ISO-8601 UTC instant when this selection was recorded.
    #[serde(default)]
    pub selected_at: Option<String>,
    /// Who set this pointer — `"cli"`, `"hook"`, ...
    #[serde(default)]
    pub source: Option<String>,
}

/// Resolve the **project-global** active unit. Used by CLI entry
/// points that are not inside a hook lifecycle.
///
/// # Errors
/// I/O or TOML parse errors bubble up.
pub fn resolve_active(project_root: &Path) -> Result<ActiveUnit, HookError> {
    if let Some(unit) = env_override() {
        return Ok(ActiveUnit::Active(unit));
    }
    if !project_root.join("knotch.toml").exists() {
        return Ok(ActiveUnit::NoProject);
    }
    read_pointer(&global_path(project_root))
}

/// Resolve the active unit **for a hook invocation**. Honors the
/// `KNOTCH_UNIT` env override, then the per-session pointer, then
/// falls back to the global pointer.
///
/// # Errors
/// I/O or TOML parse errors bubble up.
pub fn resolve_active_for_hook(
    project_root: &Path,
    session_id: &str,
) -> Result<ActiveUnit, HookError> {
    if let Some(unit) = env_override() {
        return Ok(ActiveUnit::Active(unit));
    }
    if !project_root.join("knotch.toml").exists() {
        return Ok(ActiveUnit::NoProject);
    }
    let session_path = session_path(project_root, session_id);
    if session_path.exists() {
        return read_pointer(&session_path);
    }
    read_pointer(&global_path(project_root))
}

/// Write the **project-global** pointer atomically. Passing `None`
/// encodes "no active unit".
///
/// # Errors
/// Directory creation or TOML serialization failures bubble up.
pub fn write_active(
    project_root: &Path,
    unit: Option<&UnitId>,
    source: &str,
) -> Result<(), HookError> {
    write_pointer(&global_path(project_root), unit, source)
}

/// Write a **per-session** pointer atomically. Used by the
/// `SessionStart` hook to snapshot the global pointer into a
/// session-scoped file so that later `knotch unit use` elsewhere
/// doesn't disturb the running session.
///
/// # Errors
/// See [`write_active`].
pub fn write_active_for_session(
    project_root: &Path,
    unit: Option<&UnitId>,
    session_id: &str,
    source: &str,
) -> Result<(), HookError> {
    write_pointer(&session_path(project_root, session_id), unit, source)
}

/// Remove the per-session pointer (e.g. at SessionEnd). Silent
/// success when the file is already absent.
///
/// # Errors
/// I/O errors other than `NotFound` bubble up.
pub fn clear_session(project_root: &Path, session_id: &str) -> Result<(), HookError> {
    let path = session_path(project_root, session_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Compute the project root for a given CWD: the first ancestor
/// containing `knotch.toml`, or the CWD itself if no ancestor
/// qualifies.
#[must_use]
pub fn project_root(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        if ancestor.join("knotch.toml").exists() {
            return ancestor.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

fn env_override() -> Option<UnitId> {
    // A malformed `KNOTCH_UNIT` env var should degrade to "no
    // override" rather than poison every subsequent hook call —
    // operators fix the typo and retry. `try_new` surfaces the
    // grammar violation; the `.ok()` drops it silently here.
    std::env::var(UNIT_ENV_VAR).ok().filter(|v| !v.is_empty()).and_then(|v| UnitId::try_new(v).ok())
}

fn global_path(project_root: &Path) -> PathBuf {
    project_root.join(".knotch").join("active.toml")
}

fn session_path(project_root: &Path, session_id: &str) -> PathBuf {
    project_root
        .join(".knotch")
        .join("sessions")
        .join(format!("{}.toml", sanitize_session_id(session_id)))
}

/// `session_id` values from Claude Code are UUID-ish, but we don't
/// trust them to be filesystem-safe. Replace anything that isn't
/// alphanumeric / `_` / `-` with `_`.
fn sanitize_session_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

fn read_pointer(path: &Path) -> Result<ActiveUnit, HookError> {
    if !path.exists() {
        return Ok(ActiveUnit::Uninitialized);
    }
    let raw = std::fs::read_to_string(path)?;
    let parsed: ActiveToml = toml::from_str(&raw).map_err(|e| HookError::Toml(e.to_string()))?;
    if parsed.unit.is_empty() {
        Ok(ActiveUnit::Uninitialized)
    } else {
        let unit = UnitId::try_new(&parsed.unit).map_err(|e| {
            HookError::Toml(
                format!("active.toml carries invalid unit slug {:?}: {e}", parsed.unit,),
            )
        })?;
        Ok(ActiveUnit::Active(unit))
    }
}

fn write_pointer(path: &Path, unit: Option<&UnitId>, source: &str) -> Result<(), HookError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = ActiveToml {
        unit: unit.map(|u| u.as_str().to_owned()).unwrap_or_default(),
        selected_at: Some(jiff::Timestamp::now().to_string()),
        source: Some(source.to_owned()),
    };
    let raw = toml::to_string_pretty(&body).map_err(|e| HookError::Toml(e.to_string()))?;
    crate::atomic::write(path, raw.as_bytes())?;
    Ok(())
}
