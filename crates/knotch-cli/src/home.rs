//! Home-directory resolution shared across the CLI.
//!
//! Wraps the env-var lookup so every caller (init, hook fallbacks,
//! config discovery) converges on the same platform policy: `HOME`
//! on Unix, `USERPROFILE` on Windows, `None` if neither is set.

use std::path::PathBuf;

/// Best-effort current-user home directory.
///
/// Returns `None` when neither `HOME` (Unix) nor `USERPROFILE`
/// (Windows) is set — callers decide whether that is fatal (init
/// surface) or degrades silently (hook fallbacks that pick a less
/// ideal path).
#[must_use]
pub(crate) fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}
