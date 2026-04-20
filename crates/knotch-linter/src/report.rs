//! Lint diagnostic types.

use std::{fmt, path::PathBuf};

use compact_str::CompactString;

/// Canonical rule identifier (e.g. `R1`, `R2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuleId(pub &'static str);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Severity of a lint finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Prevents build.
    Error,
    /// Visible warning.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => f.write_str("error"),
            Self::Warning => f.write_str("warning"),
        }
    }
}

/// One lint finding.
#[derive(Debug, Clone)]
pub struct Violation {
    /// Rule that produced the finding.
    pub rule: RuleId,
    /// File the finding refers to.
    pub path: PathBuf,
    /// 1-indexed line number.
    pub line: u32,
    /// 1-indexed column number.
    pub column: u32,
    /// Severity.
    pub severity: Severity,
    /// One-line human-readable message.
    pub message: CompactString,
}

impl Violation {
    /// Render as `path:line:col: [Rn] severity: message`.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{}:{}:{}: [{}] {}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.rule,
            self.severity,
            self.message,
        )
    }
}

/// Aggregated report across files.
#[derive(Debug, Default)]
pub struct LintReport {
    /// All findings, preserving scan order.
    pub violations: Vec<Violation>,
}

impl LintReport {
    /// Create an empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append findings from a single file.
    pub fn extend(&mut self, findings: impl IntoIterator<Item = Violation>) {
        self.violations.extend(findings);
    }

    /// Count of error-severity violations.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.violations.iter().filter(|v| v.severity == Severity::Error).count()
    }

    /// `true` if no findings were produced.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

impl fmt::Display for LintReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.violations.is_empty() {
            return writeln!(f, "knotch-linter: clean");
        }
        for v in &self.violations {
            writeln!(f, "{}", v.render())?;
        }
        writeln!(
            f,
            "knotch-linter: {} violation(s) ({} error(s))",
            self.violations.len(),
            self.error_count(),
        )
    }
}
