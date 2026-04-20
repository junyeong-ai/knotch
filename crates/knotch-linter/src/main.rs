//! `cargo knotch-linter` entry point.
//!
//! Walks the supplied paths, parses every `.rs` file with `syn`, and
//! runs the default rule set. Exits non-zero when any error-severity
//! violation is found.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use knotch_linter::{LintReport, default_rules, lint_file};

#[derive(Debug, Parser)]
#[command(
    name = "cargo-knotch-linter",
    version,
    about = "Knotch-specific lints: direct log writes + forbidden naming"
)]
struct Cli {
    /// Cargo injects this as the first positional on `cargo knotch-linter`.
    /// Parsed and discarded.
    #[arg(hide = true, value_parser = ["knotch-linter"], default_value = "knotch-linter")]
    _cargo_subcommand: String,

    /// Paths to scan. Directories are walked recursively. Defaults to
    /// the current working directory.
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Skip these directory names while walking (in addition to the
    /// always-skipped `target`, `.git`, `.knotch`).
    #[arg(long = "exclude")]
    exclude: Vec<String>,

    /// Emit warnings but do not fail the process.
    #[arg(long = "no-fail")]
    no_fail: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rules = default_rules();
    let mut report = LintReport::new();
    let exclude: Vec<String> = cli.exclude;

    for root in &cli.paths {
        if let Err(err) = walk(root, &exclude, &mut |path| match lint_file(path, &rules) {
            Ok(findings) => report.extend(findings),
            Err(err) => {
                eprintln!("knotch-linter: {err}");
            }
        }) {
            eprintln!("knotch-linter: walk failed at {}: {err}", root.display());
            return ExitCode::from(2);
        }
    }

    print!("{report}");

    if cli.no_fail || report.error_count() == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn walk(root: &Path, exclude: &[String], visit: &mut dyn FnMut(&Path)) -> std::io::Result<()> {
    if root.is_file() {
        if is_rust_source(root) {
            visit(root);
        }
        return Ok(());
    }
    let meta = std::fs::metadata(root)?;
    if !meta.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if skip_dir(&path, exclude) {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_dir() {
            walk(&path, exclude, visit)?;
        } else if ty.is_file() && is_rust_source(&path) {
            visit(&path);
        }
    }
    Ok(())
}

fn skip_dir(path: &Path, exclude: &[String]) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    if matches!(name, "target" | ".git" | ".knotch" | ".venv" | "node_modules") {
        return true;
    }
    exclude.iter().any(|e| e == name)
}

fn is_rust_source(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("rs")
}
