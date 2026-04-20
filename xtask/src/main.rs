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
    // Two gates, composed:
    //   (1) `file:line` citations in `.claude/rules/*.md` still point
    //       at real lines.
    //   (2) Every `EventBody<W>` variant in
    //       `crates/knotch-kernel/src/event.rs` has a row in the
    //       `event-ownership.md` Owner table + Opt-in matrix, and in
    //       the `preconditions.md` Variant → check table.
    //
    // Both together keep the rule files structurally in sync with the
    // kernel surface (constitution §VII — any rule that can be a CI
    // gate is).
    let citation_failures = check_citations()?;
    let parity_failures = check_variant_parity()?;

    let total = citation_failures.len() + parity_failures.len();
    if total == 0 {
        println!("xtask docs-lint: ok");
        return Ok(());
    }
    for f in citation_failures.iter().chain(parity_failures.iter()) {
        eprintln!("  {f}");
    }
    anyhow::bail!("{total} docs-lint failure(s)")
}

fn check_citations() -> anyhow::Result<Vec<String>> {
    let rules_dir = Path::new(".claude/rules");
    let Ok(entries) = std::fs::read_dir(rules_dir) else {
        return Ok(Vec::new());
    };
    let mut failures: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let body = std::fs::read_to_string(&path)?;
        for (line_no, line) in body.lines().enumerate() {
            for citation in extract_citations(line) {
                if let Err(why) = verify_citation(&citation) {
                    failures.push(format!(
                        "{}:{} — {citation}: {why}",
                        path.display(),
                        line_no + 1,
                    ));
                }
            }
        }
    }
    Ok(failures)
}

/// Variant-row parity between `EventBody<W>` and the rule-file tables
/// that document each variant's owner, emission mode, and precondition.
///
/// The check is one-directional: every variant in code must appear in
/// each table. Extra table rows are allowed (adopters may document
/// planned variants ahead of implementation).
fn check_variant_parity() -> anyhow::Result<Vec<String>> {
    let event_src = Path::new("crates/knotch-kernel/src/event.rs");
    let ownership = Path::new(".claude/rules/event-ownership.md");
    let preconditions = Path::new(".claude/rules/preconditions.md");
    if !event_src.exists() || !ownership.exists() || !preconditions.exists() {
        return Ok(Vec::new());
    }

    let variants = extract_event_body_variants(&std::fs::read_to_string(event_src)?);
    let ownership_body = std::fs::read_to_string(ownership)?;
    let preconditions_body = std::fs::read_to_string(preconditions)?;

    let owner_rows = table_variants(&ownership_body, "## Owner table");
    let optin_rows = table_variants(&ownership_body, "## Opt-in matrix");
    let precondition_rows = table_variants(&preconditions_body, "## Variant → check");

    let mut failures = Vec::new();
    for gate in [
        (ownership, "Owner table", &owner_rows),
        (ownership, "Opt-in matrix", &optin_rows),
        (preconditions, "Variant → check", &precondition_rows),
    ] {
        let (path, table, rows) = gate;
        let missing: Vec<&str> =
            variants.iter().filter(|v| !rows.contains(v.as_str())).map(String::as_str).collect();
        if !missing.is_empty() {
            failures.push(format!(
                "{}: missing {} row(s) for: {}",
                path.display(),
                table,
                missing.join(", ")
            ));
        }
    }
    Ok(failures)
}

/// Extract variant names from the `pub enum EventBody<W: WorkflowKind>`
/// block. Recognises both struct-style (`UnitCreated {`) and unit-style
/// (`UnitCreated,`) declarations at four-space indent.
fn extract_event_body_variants(source: &str) -> Vec<String> {
    let Some(start) = source.find("pub enum EventBody<W") else {
        return Vec::new();
    };
    let rest = &source[start..];
    let Some(brace) = rest.find('{') else {
        return Vec::new();
    };
    let mut depth = 0_i32;
    let mut end = 0_usize;
    for (i, c) in rest[brace..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = brace + i;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return Vec::new();
    }
    let body = &rest[brace + 1..end];

    let mut variants = Vec::new();
    let mut inner_depth = 0_i32;
    for raw_line in body.lines() {
        let pre_depth = inner_depth;
        for c in raw_line.chars() {
            match c {
                '{' => inner_depth += 1,
                '}' => inner_depth -= 1,
                _ => {}
            }
        }
        if pre_depth > 0 {
            continue;
        }
        let line = raw_line.trim_start();
        if line.starts_with("///") || line.starts_with("//") || line.is_empty() {
            continue;
        }
        let indent = raw_line.len() - line.len();
        if indent != 4 {
            continue;
        }
        let token_end =
            line.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(line.len());
        if token_end == 0 {
            continue;
        }
        let name = &line[..token_end];
        if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let suffix = line[token_end..].trim_start();
        if suffix.starts_with('{') || suffix.starts_with('(') || suffix.starts_with(',') {
            variants.push(name.to_owned());
        }
    }
    variants
}

/// Extract first-column identifiers from a Markdown table immediately
/// following `heading` (e.g. `## Owner table`). Identifiers are read
/// from backtick-wrapped spans in the first cell — rows without
/// backticked names (or the header / separator rows) are skipped.
fn table_variants(markdown: &str, heading: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Some(start) = markdown.find(heading) else {
        return out;
    };
    let rest = &markdown[start + heading.len()..];
    for line in rest.lines() {
        if line.starts_with("## ") {
            break;
        }
        if !line.starts_with('|') {
            continue;
        }
        let first = line[1..].split('|').next().unwrap_or("").trim();
        if let Some(name) = first.strip_prefix('`').and_then(|s| s.split_once('`')).map(|(n, _)| n)
        {
            out.insert(name.to_owned());
        }
    }
    out
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
        if inner.starts_with("crates/")
            && inner.contains(".rs:")
            && let Some((path, rest)) = inner.split_once(':')
            && rest.chars().all(|c| c.is_ascii_digit())
        {
            out.push(format!("{path}:{rest}"));
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
