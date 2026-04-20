//! Phase 7 exit criterion:
//!
//! - Running the linter on its own sources produces zero findings
//!   (workspace self-lint).
//! - Intentional violation fixtures produce ≥ 1 finding.

use std::path::{Path, PathBuf};

use knotch_linter::{LintReport, default_rules, lint_file};

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn rec(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "target" | ".git" | "fixtures") {
                    continue;
                }
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => rec(&path, out),
                Ok(t) if t.is_file()
                    && path.extension().and_then(|e| e.to_str()) == Some("rs") =>
                {
                    out.push(path);
                }
                _ => {}
            }
        }
    }
    rec(root, &mut out);
    out
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture present")
}

fn lint_source(contents: &str) -> Vec<knotch_linter::Violation> {
    // Drop the fixture into a tempdir with no ancestor Cargo.toml so
    // crate-name detection returns None and the allowlist doesn't
    // suppress findings.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture.rs");
    std::fs::write(&path, contents).expect("write fixture");
    lint_file(&path, &default_rules()).expect("lint fixture")
}

#[test]
fn self_lint_is_clean() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = crate_dir.join("src");
    let rules = default_rules();
    let mut report = LintReport::new();
    for file in walk(&src_dir) {
        match lint_file(&file, &rules) {
            Ok(v) => report.extend(v),
            Err(e) => panic!("parse failure on {}: {e}", file.display()),
        }
    }
    assert!(report.is_clean(), "expected clean self-lint, got:\n{report}");
}

#[test]
fn fixture_direct_log_write_produces_violation() {
    let src = fixture("direct_write.rs.txt");
    let findings = lint_source(&src);
    assert!(
        findings.iter().any(|v| v.rule.0 == "R1"),
        "expected an R1 violation, got {findings:?}"
    );
}

#[test]
fn fixture_forbidden_name_produces_violation() {
    let src = fixture("forbidden_name.rs.txt");
    let findings = lint_source(&src);
    let r2_count = findings.iter().filter(|v| v.rule.0 == "R2").count();
    assert!(
        r2_count >= 3,
        "expected >=3 R2 violations (CacheManager, RequestHandler, JobProcessor), got {findings:?}"
    );
}
