//! Non-release automation for the knotch workspace.
//!
//! Release publishing is handled by `cargo release --workspace`; xtask
//! owns the jobs that don't fit there — running the full local CI
//! pipeline, regenerating public-API baselines, and linting the
//! rule files for broken `file:line` citations.

use std::{
    path::Path,
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "xtask", version, about = "Knotch workspace automation")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Regenerate `docs/public_api/*.baseline` for every publishable
    /// crate using nightly `cargo public-api`.
    PublicApi,
    /// Verify every `.claude/rules/*.md` file's `file:line` citations
    /// still point at real lines in the workspace.
    DocsLint,
    /// Mirror `.claude/skills/` into `plugins/knotch/skills/` with
    /// the `knotch-` prefix stripped (so `/knotch:query` rather than
    /// `/knotch:knotch-query`). Rebuilds the destination — do not
    /// hand-edit `plugins/knotch/skills/`.
    PluginSync {
        /// Print the planned changes without writing any files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the complete CI pipeline locally.
    Ci,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Task::PublicApi => public_api(),
        Task::DocsLint => docs_lint(),
        Task::PluginSync { dry_run } => plugin_sync(dry_run),
        Task::Ci => ci(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err:#}");
            ExitCode::FAILURE
        }
    }
}

const PUBLISHABLE_CRATES: &[&str] = &[
    "knotch-kernel",
    "knotch-proto",
    "knotch-derive",
    "knotch-storage",
    "knotch-lock",
    "knotch-vcs",
    "knotch-workflow",
    "knotch-schema",
    "knotch-observer",
    "knotch-reconciler",
    "knotch-query",
    "knotch-tracing",
    "knotch-testing",
    "knotch-linter",
    "knotch-agent",
    "knotch-frontmatter",
    "knotch-adr",
];

fn public_api() -> anyhow::Result<()> {
    for c in PUBLISHABLE_CRATES {
        let out = std::fs::File::create(format!("docs/public_api/{c}.baseline"))?;
        let mut cmd = Command::new("cargo");
        cmd.args([
            "+nightly",
            "public-api",
            "--manifest-path",
            &format!("crates/{c}/Cargo.toml"),
            "--simplified",
        ])
        .stdout(out);
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("cargo public-api failed for {c}");
        }
    }
    Ok(())
}

fn docs_lint() -> anyhow::Result<()> {
    // Scan every `.claude/rules/*.md` for `path/file.rs:LINE` citations
    // and verify each referenced line still exists. Pure Rust — no
    // external tools.
    let rules_dir = Path::new(".claude/rules");
    let Ok(entries) = std::fs::read_dir(rules_dir) else {
        println!("xtask docs-lint: no .claude/rules directory — skipping");
        return Ok(());
    };
    let mut failures: Vec<String> = Vec::new();
    // Matches `crates/<crate>/src/<path>.rs:<line>` inline-code style.
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let body = std::fs::read_to_string(&path)?;
        for (line_no, line) in body.lines().enumerate() {
            for citation in extract_citations(line) {
                match verify_citation(&citation) {
                    Ok(()) => {}
                    Err(why) => failures.push(format!(
                        "{}:{} — {citation}: {why}",
                        path.display(),
                        line_no + 1,
                    )),
                }
            }
        }
    }
    if failures.is_empty() {
        println!("xtask docs-lint: ok");
        Ok(())
    } else {
        for f in &failures {
            eprintln!("  {f}");
        }
        anyhow::bail!("{} broken citation(s)", failures.len())
    }
}

fn extract_citations(line: &str) -> Vec<String> {
    // Conservative extraction: accepts `crates/<crate>/...:<line>`
    // inside inline-code spans delimited by backticks.
    let mut out = Vec::new();
    let mut chars = line.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '`' {
            continue;
        }
        let rest = &line[i + 1..];
        let end = rest.find('`').unwrap_or(rest.len());
        let inner = &rest[..end];
        if inner.starts_with("crates/") && inner.contains(".rs:") {
            if let Some((path, rest)) = inner.split_once(':') {
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    out.push(format!("{path}:{rest}"));
                }
            }
        }
        for _ in 0..end + 1 {
            chars.next();
        }
    }
    out
}

fn verify_citation(citation: &str) -> Result<(), String> {
    let (path, line_s) =
        citation.rsplit_once(':').ok_or_else(|| "malformed citation".to_owned())?;
    let line: usize = line_s.parse().map_err(|_| "line is not a number".to_owned())?;
    let body = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    if body.lines().count() < line {
        return Err(format!("{path} has {} lines; citation points past EOF", body.lines().count()));
    }
    Ok(())
}

fn plugin_sync(dry_run: bool) -> anyhow::Result<()> {
    let plugin_root = Path::new("plugins/knotch");
    if !plugin_root.exists() {
        anyhow::bail!("plugins/knotch/ not found — run from workspace root");
    }

    // 1. Manifest must already exist — `plugin-sync` never fabricates it.
    let manifest = plugin_root.join(".claude-plugin").join("plugin.json");
    if !manifest.exists() {
        anyhow::bail!("{} missing — create the plugin manifest before syncing", manifest.display());
    }

    // 2. Plan the skill sync. Each `.claude/skills/knotch-<name>/` maps to
    //    `plugins/knotch/skills/<name>/` — the `knotch-` prefix is stripped so plugin-mode
    //    invocation uses the clean namespaced name `/knotch:<name>`.
    let skills_src = Path::new(".claude/skills");
    let skills_dst = plugin_root.join("skills");
    let plan = plan_skill_rename(skills_src)?;

    if dry_run {
        println!("-- dry-run: planned changes --");
        println!("dst:  {}", skills_dst.display());
        for (src_name, dst_name) in &plan {
            if src_name == dst_name {
                println!("  copy    {src_name}");
            } else {
                println!("  rename  {src_name} -> {dst_name}");
            }
        }
        return Ok(());
    }

    // 3. Rebuild the destination. `plugins/knotch/skills/` is a derived artifact — hand-edits
    //    are lost by design.
    rebuild_dir(&skills_dst)?;
    std::fs::write(
        skills_dst.join("README.md"),
        "# skills\n\n\
         Generated by `cargo xtask plugin-sync` — do not hand-edit.\n\
         Sources live under `.claude/skills/knotch-<name>/SKILL.md`.\n",
    )?;

    for (src_name, dst_name) in &plan {
        let src = skills_src.join(src_name);
        let dst = skills_dst.join(dst_name);
        copy_dir_recursive(&src, &dst)?;
    }

    println!("plugin synced: {} skill(s) → {}", plan.len(), skills_dst.display());
    Ok(())
}

/// Enumerate skill directories under `src` and return `(source,
/// dest)` pairs with the `knotch-` prefix stripped from the dest.
fn plan_skill_rename(src: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut plan = Vec::new();
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let src_name = entry.file_name().to_string_lossy().into_owned();
        let dst_name = src_name.strip_prefix("knotch-").unwrap_or(&src_name).to_owned();
        plan.push((src_name, dst_name));
    }
    plan.sort();
    Ok(plan)
}

fn rebuild_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn ci() -> anyhow::Result<()> {
    // Ordered so fast gates run first and cheap failures stop the
    // pipeline before expensive ones.
    let steps: [&[&str]; 7] = [
        &["cargo", "fmt", "--all", "--check"],
        &["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        &["cargo", "knotch-linter"],
        &["cargo", "nextest", "run", "--workspace"],
        &["cargo", "test", "--workspace", "--doc"],
        &["cargo", "deny", "check"],
        &["cargo", "machete"],
    ];
    for step in steps {
        let mut cmd = Command::new(step[0]);
        cmd.args(&step[1..]);
        run(cmd)?;
    }
    Ok(())
}

fn run(mut cmd: Command) -> anyhow::Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("command failed: {cmd:?} ({status})");
    }
    Ok(())
}
