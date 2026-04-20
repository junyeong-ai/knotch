//! `knotch init` — scaffold a workspace.
//!
//! Creates `knotch.toml`, `state/`, and `.knotch/`. Optional flags:
//!
//! - `--with-hooks` merges the knotch hook block into a Claude Code settings file chosen
//!   by `--hook-target`.
//! - `--demo` pre-populates a sample `demo` unit and makes it active so a new user can
//!   run `knotch show demo` immediately.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
use clap::{Args as ClapArgs, ValueEnum};
use knotch_agent::active::write_active;
use knotch_kernel::{
    AppendMode, Causation, Proposal, Repository, Scope, UnitId, WorkflowKind,
    event::{ArtifactList, EventBody},
};
use knotch_workflow::ConfigWorkflow;
use serde_json::{Value, json};

use crate::{cmd::OutputMode, config::Config};

/// Where to install the knotch hook block.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum HookTarget {
    /// Project-shared: `.claude/settings.json` (committed).
    Project,
    /// Personal: `.claude/settings.local.json` (gitignored).
    Local,
    /// User-global: `~/.claude/settings.json`.
    User,
}

/// `knotch init` arguments.
#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Overwrite an existing `knotch.toml` if present.
    #[arg(long)]
    pub force: bool,
    /// Also install Claude Code hooks into the chosen settings file.
    #[arg(long)]
    pub with_hooks: bool,
    /// Where to write the hook block. Ignored when `--with-hooks`
    /// is absent.
    #[arg(long, value_enum, default_value_t = HookTarget::Project)]
    pub hook_target: HookTarget,
    /// Pre-populate a `demo` unit with a UnitCreated + PhaseCompleted
    /// pair so that `knotch show demo` works immediately.
    #[arg(long)]
    pub demo: bool,
    /// Also merge the opt-in hook blocks (UserPromptSubmit refresh,
    /// SessionEnd finalize) into the chosen settings file. Without
    /// this flag the example file is still written for reference.
    #[arg(long)]
    pub include_optional: bool,
}

/// Run the init command.
///
/// # Errors
/// Returns an error if the target directory is not writable or if
/// the config file already exists without `--force`.
pub(crate) async fn run(config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()> {
    let cfg_path = config.config_path();
    let existed = cfg_path.exists();
    if existed && !args.force {
        return Err(anyhow!("{} already exists — pass --force to overwrite", cfg_path.display()));
    }

    tokio::fs::create_dir_all(&config.state_dir)
        .await
        .with_context(|| format!("create state dir {}", config.state_dir.display()))?;
    tokio::fs::create_dir_all(config.root.join(".knotch"))
        .await
        .with_context(|| format!("create {}/.knotch", config.root.display()))?;

    let body = default_config_toml(&relative_state_dir(&config.root, &config.state_dir));
    tokio::fs::write(&cfg_path, body)
        .await
        .with_context(|| format!("write {}", cfg_path.display()))?;

    let hooks_written = if args.with_hooks {
        install_hooks(&config.root, args.hook_target, args.include_optional).await?
    } else {
        None
    };
    let example_written =
        if args.with_hooks { Some(write_optional_example(&config.root).await?) } else { None };

    // Always wire the `.knotch/` runtime dir into .gitignore and
    // ship a README explaining its role. Idempotent — re-runs don't
    // duplicate entries.
    ensure_gitignored_knotch(&config.root).await?;
    write_knotch_readme(&config.root).await?;

    let demo_written = if args.demo {
        populate_demo(config).await?;
        true
    } else {
        false
    };

    match out {
        OutputMode::Human => {
            println!(
                "initialized knotch workspace:\n  root:       {}\n  config:     {}\n  state dir:  {}",
                config.root.display(),
                cfg_path.display(),
                config.state_dir.display(),
            );
            if existed {
                println!("(existing knotch.toml was overwritten)");
            }
            if let Some(path) = &hooks_written {
                println!("  hooks:      {}", path.display());
            }
            if let Some(path) = &example_written {
                println!("  example:    {}", path.display());
            }
            if demo_written {
                println!("  demo unit:  {}/demo", config.state_dir.display());
            }
            print_next_steps(&NextStepsContext {
                demo: demo_written,
                with_hooks: hooks_written.is_some(),
            });
        }
        OutputMode::Json => {
            let value = json!({
                "event": "init",
                "root": config.root.display().to_string(),
                "config": cfg_path.display().to_string(),
                "state_dir": config.state_dir.display().to_string(),
                "overwritten": existed,
                "hooks_written": hooks_written.as_ref().map(|p| p.display().to_string()),
                "optional_example": example_written.as_ref().map(|p| p.display().to_string()),
                "demo": demo_written,
            });
            println!("{value}");
        }
    }
    Ok(())
}

fn relative_state_dir(root: &Path, state_dir: &Path) -> PathBuf {
    state_dir.strip_prefix(root).map(Path::to_path_buf).unwrap_or_else(|_| state_dir.to_path_buf())
}

fn default_config_toml(state_dir: &Path) -> String {
    format!(
        concat!(
            "# knotch workspace configuration\n",
            "# Generated by `knotch init`. Hand-edit freely — `knotch` re-reads this file on every invocation.\n\n",
            "# Directory (relative to this file) where per-unit logs live.\n",
            "state_dir = \"{state_dir}\"\n\n",
            "# Wire-format schema version this workspace expects.\n",
            "schema_version = {schema}\n\n",
            "# Hook guard policy. `block` (exit 2), `warn` (context\n",
            "# injection, default), or `off` (silent). Applies to\n",
            "# `git push --force` / `reset --hard` / `branch -D` /\n",
            "# `checkout --` / `clean -f` / `rebase -i|--root`.\n",
            "[guard]\n",
            "rewrite = \"warn\"\n\n",
            "# --- workflow -------------------------------------------------\n",
            "# The lifecycle this project runs. Edit phases, gates,\n",
            "# required_phases, terminal_statuses, or known_statuses\n",
            "# below to tailor the ledger to your domain — every field\n",
            "# is read by the `knotch` binary at startup.\n",
            "#\n",
            "# Leaving this section untouched gives you the canonical\n",
            "# knotch shape.\n",
            "[workflow]\n",
            "name = \"knotch\"\n",
            "schema_version = 1\n",
            "default_scope = \"standard\"\n",
            "terminal_statuses = [\"archived\", \"abandoned\", \"superseded\", \"deprecated\"]\n",
            "known_statuses = [\n",
            "    \"draft\", \"in_progress\", \"in_review\", \"shipped\",\n",
            "    \"archived\", \"abandoned\", \"superseded\", \"deprecated\",\n",
            "]\n",
            "\n",
            "[workflow.required_phases]\n",
            "tiny = [\"specify\", \"build\", \"ship\"]\n",
            "standard = [\"specify\", \"plan\", \"build\", \"review\", \"ship\"]\n",
            "\n",
            "[[workflow.phases]]\n",
            "id = \"specify\"\n",
            "\n",
            "[[workflow.phases]]\n",
            "id = \"plan\"\n",
            "\n",
            "[[workflow.phases]]\n",
            "id = \"build\"\n",
            "\n",
            "[[workflow.phases]]\n",
            "id = \"review\"\n",
            "\n",
            "[[workflow.phases]]\n",
            "id = \"ship\"\n",
            "\n",
            "[[workflow.gates]]\n",
            "id = \"g0-scope\"\n",
            "prerequisites = []\n",
            "\n",
            "[[workflow.gates]]\n",
            "id = \"g1-clarify\"\n",
            "prerequisites = [\"g0-scope\"]\n",
            "\n",
            "[[workflow.gates]]\n",
            "id = \"g2-plan\"\n",
            "prerequisites = [\"g0-scope\", \"g1-clarify\"]\n",
            "\n",
            "[[workflow.gates]]\n",
            "id = \"g3-review\"\n",
            "prerequisites = [\"g0-scope\", \"g1-clarify\", \"g2-plan\"]\n",
            "\n",
            "[[workflow.gates]]\n",
            "id = \"g4-drift\"\n",
            "prerequisites = [\"g0-scope\", \"g1-clarify\", \"g2-plan\", \"g3-review\"]\n",
        ),
        state_dir = state_dir.display(),
        schema = knotch_proto::SCHEMA_VERSION,
    )
}

struct NextStepsContext {
    demo: bool,
    with_hooks: bool,
}

fn print_next_steps(ctx: &NextStepsContext) {
    println!();
    println!("next steps:");
    if ctx.demo {
        println!("  knotch show demo                  # peek at the pre-populated unit");
        println!("  knotch unit list                  # list every known unit");
    } else {
        println!("  knotch unit init my-feature       # create your first unit");
        println!("  knotch unit use my-feature        # make it the active unit");
    }
    println!("  knotch current                    # confirm the active unit");
    println!();
    if ctx.with_hooks {
        println!("  # Claude Code hooks are now wired up — git commits will auto-record");
        println!("  # MilestoneShipped events on the active unit.");
    } else {
        println!("  # re-run with --with-hooks to auto-record commits from Claude Code.");
    }
    println!("  # docs: https://knotch.dev  /  CLAUDE.md at the repo root");
}

async fn populate_demo(config: &Config) -> anyhow::Result<()> {
    let unit = UnitId::try_new("demo").expect("`demo` is a valid slug");
    let repo = config.build_repository()?;
    append_body::<ConfigWorkflow, _>(
        &repo,
        &unit,
        EventBody::UnitCreated { scope: Scope::Standard },
    )
    .await?;
    let specify = repo
        .workflow()
        .parse_phase("specify")
        .ok_or_else(|| anyhow!("workflow is missing a `specify` phase; `knotch init --demo` expects the canonical shape"))?;
    append_body::<ConfigWorkflow, _>(
        &repo,
        &unit,
        EventBody::PhaseCompleted { phase: specify, artifacts: ArtifactList::default() },
    )
    .await?;
    write_active(&config.root, Some(&unit), "init-demo")
        .map_err(|e| anyhow!("write active: {e}"))?;
    Ok(())
}

async fn append_body<W, R>(repo: &R, unit: &UnitId, body: EventBody<W>) -> anyhow::Result<()>
where
    W: knotch_kernel::WorkflowKind,
    W::Extension: Default,
    R: Repository<W>,
{
    let proposal = Proposal {
        causation: Causation::cli("init-demo"),
        extension: <W::Extension as Default>::default(),
        body,
        supersedes: None,
    };
    repo.append(unit, vec![proposal], AppendMode::AllOrNothing).await?;
    Ok(())
}

async fn install_hooks(
    root: &Path,
    target: HookTarget,
    include_optional: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let path = match target {
        HookTarget::Project => root.join(".claude").join("settings.json"),
        HookTarget::Local => root.join(".claude").join("settings.local.json"),
        HookTarget::User => home_dir()?.join(".claude").join("settings.json"),
    };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let existing: Value = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parse existing {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => {
            return Err(anyhow::Error::new(err).context(format!("read {}", path.display())));
        }
    };
    let mut merged = merge_hooks(existing, knotch_mandatory_hook_block());
    if include_optional {
        merged = merge_hooks(merged, knotch_optional_hook_block());
    }
    let body = serde_json::to_string_pretty(&merged)?;
    tokio::fs::write(&path, body).await.with_context(|| format!("write {}", path.display()))?;
    Ok(Some(path))
}

/// Write the optional-hooks example file next to the settings file.
/// Users copy blocks out of this into their real `settings.json` when
/// they decide to enable them.
async fn write_optional_example(root: &Path) -> anyhow::Result<PathBuf> {
    let path = root.join(".claude").join("knotch-optional-hooks.example.jsonc");
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, OPTIONAL_EXAMPLE_JSONC)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

const OPTIONAL_EXAMPLE_JSONC: &str = r#"// Optional knotch hooks. Copy any block below into the `hooks`
// section of your `.claude/settings.json` to enable it.
//
// These are OFF by default because they fire on every prompt /
// session end and the cost — while small — is not zero.
//
// JSONC (JSON with comments) is accepted by most tooling; strip
// comments with `jq '.'` before merging if your settings.json
// validator rejects them.
{
  "hooks": {
    // Fires on every user prompt. Re-injects the active-unit
    // context so mid-session `knotch unit use X` elsewhere is
    // picked up.
    "UserPromptSubmit": [
      {
        "hooks": [
          { "type": "command", "command": "knotch hook refresh-context" }
        ]
      }
    ],

    // Fires when the session ends via /clear, logout, or other
    // exits. Surfaces reconciler-queue size so operators know to
    // drain it.
    "SessionEnd": [
      {
        "matcher": "clear|logout|other",
        "hooks": [
          { "type": "command", "command": "knotch hook finalize-session" }
        ]
      }
    ]
  }
}
"#;

fn home_dir() -> anyhow::Result<PathBuf> {
    crate::home::user_home()
        .ok_or_else(|| anyhow!("HOME / USERPROFILE env var not set — cannot resolve user settings"))
}

/// Ensure the `.knotch/` runtime directory is gitignored. Idempotent.
async fn ensure_gitignored_knotch(root: &Path) -> anyhow::Result<()> {
    let path = root.join(".gitignore");
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let already_ignored = existing.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == ".knotch"
            || trimmed == ".knotch/"
            || trimmed == "/.knotch"
            || trimmed == "/.knotch/"
    });
    if already_ignored {
        return Ok(());
    }
    let mut new = existing;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    if !new.is_empty() {
        new.push('\n');
    }
    new.push_str("# knotch runtime state (project config is knotch.toml)\n");
    new.push_str(".knotch/\n");
    tokio::fs::write(&path, new).await.with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Document the role of `.knotch/` vs `knotch.toml` for operators.
/// Idempotent — overwrites with the canonical text each run.
async fn write_knotch_readme(root: &Path) -> anyhow::Result<()> {
    let path = root.join(".knotch").join("README.md");
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    tokio::fs::write(&path, KNOTCH_DIR_README)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

const KNOTCH_DIR_README: &str = r#"# .knotch/ — knotch runtime state

This directory is **not committed**. It holds per-machine, per-session
state that knotch maintains during a workflow run. Project-wide
configuration lives in `../knotch.toml` (which is committed).

## Contents

| Entry                         | Who writes              | Commit? |
|-------------------------------|-------------------------|---------|
| `active.toml`                 | `knotch unit use`       | no      |
| `sessions/<session-id>.toml`  | `knotch hook load-context` | no   |
| `queue/<uuid>.json`           | Failed `PostToolUse` appends (auto-drained at next `SessionStart`) | no |
| `subagents/<agent-id>.json`   | `knotch hook record-subagent` | no |

## Why separated

- `knotch.toml` (committed): workflow preset, state dir, schema
  version. Every teammate starts from the same config.
- `.knotch/` (local): per-machine active-unit pointers, queued
  retries, subagent transcripts. Non-portable state.

Think `Cargo.toml` + `target/`: config vs build output.

## Regenerate

Delete the directory at any time. `knotch init` will re-create it;
`knotch hook load-context` fills per-session state on the next Claude
Code session.
"#;

/// Mandatory knotch hook block. Always merged into settings when
/// `--with-hooks` is set.
fn knotch_mandatory_hook_block() -> Value {
    json!({
        "hooks": {
            "SessionStart": [
                {
                    "matcher": "",
                    "hooks": [{"type": "command", "command": "knotch hook load-context"}]
                }
            ],
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        {"type": "command", "if": "Bash(git commit *)",      "command": "knotch hook check-commit"},
                        {"type": "command", "if": "Bash(git push --force*)", "command": "knotch hook guard-rewrite"},
                        {"type": "command", "if": "Bash(git push -f *)",     "command": "knotch hook guard-rewrite"},
                        {"type": "command", "if": "Bash(git reset --hard*)", "command": "knotch hook guard-rewrite"},
                        {"type": "command", "if": "Bash(git branch -D *)",   "command": "knotch hook guard-rewrite"}
                    ]
                }
            ],
            "PostToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        {"type": "command", "if": "Bash(git commit *)", "command": "knotch hook verify-commit"},
                        {"type": "command", "if": "Bash(git revert *)", "command": "knotch hook record-revert"}
                    ]
                }
            ],
            "SubagentStop": [
                {
                    "matcher": "",
                    "hooks": [{"type": "command", "command": "knotch hook record-subagent"}]
                }
            ]
        }
    })
}

/// Opt-in knotch hook block. Merged into settings only when the
/// caller passes `--include-optional`; otherwise lives in
/// `.claude/knotch-optional-hooks.example.jsonc` as documentation.
fn knotch_optional_hook_block() -> Value {
    json!({
        "hooks": {
            "UserPromptSubmit": [
                {"hooks": [{"type": "command", "command": "knotch hook refresh-context"}]}
            ],
            "SessionEnd": [
                {"matcher": "clear|logout|other", "hooks": [{"type": "command", "command": "knotch hook finalize-session"}]}
            ]
        }
    })
}

/// Merge the knotch hook block into an existing settings object.
///
/// This is a **signature-based replace**, not a diff:
///
/// 1. Every entry in `existing.hooks.*[].hooks` whose `command` starts with `"knotch hook
///    "` or `"knotch "` is removed. Empty hook_entry groups and empty event arrays
///    collapse.
/// 2. The fresh knotch block is appended verbatim.
///
/// Consequence: re-running `knotch init --with-hooks` after editing
/// a knotch-managed hook (e.g. raising its timeout) never leaves
/// duplicates — the previous version is stripped before re-insert.
/// Non-knotch hook_entrys (prettier, custom scripts, etc.) are left
/// alone.
fn merge_hooks(existing: Value, new: Value) -> Value {
    let stripped = strip_knotch_managed(existing);
    inject_block(stripped, new)
}

/// Remove every knotch-managed hook_entry from a settings-shaped `Value`.
fn strip_knotch_managed(value: Value) -> Value {
    let Value::Object(mut obj) = value else {
        return Value::Object(serde_json::Map::new());
    };
    let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Value::Object(obj);
    };
    let event_names: Vec<String> = hooks.keys().cloned().collect();
    for event in event_names {
        let Some(arr) = hooks.get_mut(&event).and_then(Value::as_array_mut) else {
            continue;
        };
        // Within each hook_entry, strip individual knotch commands.
        for hook_entry in arr.iter_mut() {
            if let Some(hlist) = hook_entry.get_mut("hooks").and_then(Value::as_array_mut) {
                hlist.retain(|h| !is_knotch_managed(h));
            }
        }
        // Drop hook_entrys whose hooks array ended up empty.
        arr.retain(|h| {
            h.get("hooks").and_then(Value::as_array).is_none_or(|list| !list.is_empty())
        });
        // Drop events whose hook_entry array is empty.
        if arr.is_empty() {
            hooks.remove(&event);
        }
    }
    Value::Object(obj)
}

/// A hook_entry is knotch-managed when its `command` is dispatched
/// through the knotch binary. Both `knotch hook <sub>` and bare
/// `knotch <sub>` count — the latter appears if a future release
/// collapses the `hook` subgroup.
fn is_knotch_managed(hook_entry: &Value) -> bool {
    hook_entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.starts_with("knotch hook ") || c.starts_with("knotch "))
}

/// Append a fresh hook block on top of a (pre-stripped) settings map.
fn inject_block(existing: Value, new: Value) -> Value {
    let mut out = match existing {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let Some(new_obj) = new.as_object() else {
        return Value::Object(out);
    };
    let Some(new_hooks) = new_obj.get("hooks").and_then(Value::as_object) else {
        return Value::Object(out);
    };
    let existing_hooks = out
        .entry("hooks".to_owned())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("hooks is object");
    for (event, new_val) in new_hooks {
        let arr = existing_hooks
            .entry(event.clone())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("event entry is array");
        if let Some(new_arr) = new_val.as_array() {
            for item in new_arr {
                arr.push(item.clone());
            }
        }
    }
    Value::Object(out)
}
