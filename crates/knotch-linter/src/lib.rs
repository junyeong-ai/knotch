//! AST-based lint checks for knotch repositories.
//!
//! Exposed as a library so CI can drive it programmatically and so
//! the CLI binary (`cargo-knotch-linter`) stays thin.
//!
//! Rules currently enforced:
//!
//! - `R1 DirectLogWrite` — blocks direct writes to knotch log files (anything writing
//!   `log.jsonl` or `.resume-cache.json`) from outside the allowlisted adapter crates.
//! - `R2 ForbiddenName` — rejects identifiers ending in `Helper`, `Util`, `Manager`,
//!   `Handler`, `Processor`, or `Impl` (per `knotch-v1-final-plan.md §16.5`).

pub mod report;
pub mod rules;

use std::path::{Path, PathBuf};

pub use self::report::{LintReport, RuleId, Severity, Violation};

/// Shared context passed to every rule.
#[derive(Debug, Clone)]
pub struct LintContext {
    /// Relative path of the file being scanned.
    pub path: PathBuf,
    /// Crate the file belongs to (detected from `Cargo.toml` ancestor).
    pub crate_name: Option<String>,
}

/// Trait implemented by every rule.
pub trait Rule: Send + Sync {
    /// Stable rule id (`R1`, `R2`, …).
    fn id(&self) -> RuleId;

    /// One-line description of what the rule enforces.
    fn description(&self) -> &'static str;

    /// Run the rule against a parsed file.
    fn check(&self, ctx: &LintContext, file: &syn::File) -> Vec<Violation>;
}

/// Run every registered rule against a single file.
///
/// # Errors
/// Returns `LintError::Parse` when the source is not valid Rust;
/// `LintError::Io` on read failure.
pub fn lint_file(path: &Path, rules: &[Box<dyn Rule>]) -> Result<Vec<Violation>, LintError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| LintError::Io { path: path.into(), source: e })?;
    let file =
        syn::parse_file(&source).map_err(|e| LintError::Parse { path: path.into(), source: e })?;
    let ctx = LintContext { path: path.to_path_buf(), crate_name: detect_crate_name(path) };
    let mut all = Vec::new();
    for rule in rules {
        all.extend(rule.check(&ctx, &file));
    }
    Ok(all)
}

fn detect_crate_name(path: &Path) -> Option<String> {
    let mut cursor = path.parent()?;
    loop {
        let candidate = cursor.join("Cargo.toml");
        if candidate.exists() {
            return read_package_name(&candidate);
        }
        cursor = cursor.parent()?;
    }
}

fn read_package_name(cargo_toml: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(cargo_toml).ok()?;
    let mut in_package = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        // Match lines like: name = "foo" / name="foo"
        if let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            let rest = rest.strip_prefix('=')?.trim_start();
            let rest = rest.strip_prefix('"')?;
            let end = rest.find('"')?;
            return Some(rest[..end].to_owned());
        }
    }
    None
}

/// Errors raised by the linter driver.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LintError {
    /// File read failed.
    #[error("failed to read {path:?}")]
    Io {
        /// Offending path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Rust parser rejected the source.
    #[error("failed to parse {path:?}")]
    Parse {
        /// Offending path.
        path: PathBuf,
        /// Underlying syn error.
        #[source]
        source: syn::Error,
    },
}

/// Build the default rule registry.
#[must_use]
pub fn default_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(rules::DirectLogWriteRule::default()),
        Box::new(rules::ForbiddenNameRule::default()),
        Box::new(rules::KernelNoIoRule::default()),
    ]
}
