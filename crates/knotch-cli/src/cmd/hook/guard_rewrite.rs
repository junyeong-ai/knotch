//! PreToolUse(history-rewriting git ops) — policy-aware guard.
//!
//! Matched commands (token-level, order-independent):
//!
//! | Subcommand | Destructive when                              | Exempt                         |
//! |------------|-----------------------------------------------|--------------------------------|
//! | `push`     | `--force` / `-f` anywhere in args             | `--force-with-lease[=<ref>]`   |
//! | `reset`    | `--hard` anywhere                              | `--soft`, `--mixed` (default)  |
//! | `branch`   | `-D` / `--delete-force`                        | `-d` (regular delete)          |
//! | `checkout` | `--` (file-path mode, discards working copy)  | branch-switch form             |
//! | `clean`    | `-f` / `--force`                               | dry-run                        |
//! | `rebase`   | `-i` / `--interactive` / `--root`              | regular `git rebase <upstream>`|
//!
//! Policy (`config.guard.rewrite`):
//!
//! - `block` — exit 2, agent cannot proceed.
//! - `warn` (default) — inject warning into context, let it run.
//! - `off`  — silent no-op regardless of command.

use knotch_agent::{
    HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
};
use knotch_workflow::ConfigWorkflow;

use crate::config::{Config, GuardPolicy};

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    if config.guard.rewrite == GuardPolicy::Off {
        return Ok(HookOutput::Continue);
    }
    let Some(command) = input.bash_command() else {
        return Ok(HookOutput::Continue);
    };
    if !is_destructive(command) {
        return Ok(HookOutput::Continue);
    }
    let root = project_root(&input.cwd);
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        _ => return Ok(HookOutput::Continue),
    };
    let repo = config.build_repository()?;
    let raw = knotch_agent::guard::rewrite::<ConfigWorkflow, _>(&repo, &unit, command).await?;

    // Policy translates a `Block` into either a real block (exit 2)
    // or an advisory context injection.
    Ok(match (config.guard.rewrite, raw) {
        (GuardPolicy::Off, _) => HookOutput::Continue,
        (_, HookOutput::Continue | HookOutput::Context(_)) => HookOutput::Continue,
        (GuardPolicy::Block, blocked) => blocked,
        (GuardPolicy::Warn, HookOutput::Block { reason }) => HookOutput::Context(format!(
            "⚠ knotch guard: {reason} (policy=warn — running anyway; \
             set `[guard] rewrite = \"block\"` in knotch.toml to enforce)"
        )),
        (GuardPolicy::Warn, other) => other,
    })
}

/// Decide whether `cmd` is a history-rewriting / data-destroying
/// git operation worth guarding.
///
/// Token-walk (not prefix-scan) so flags at the end
/// (`git push origin main --force`) and safe variants
/// (`--force-with-lease`) behave correctly.
fn is_destructive(cmd: &str) -> bool {
    // A bash command can chain multiple git invocations via `&&`,
    // `||`, `;`, or `|` — every fragment has to be checked
    // independently or a destructive second half hides behind a
    // benign first half (`git pull && git push --force`).
    split_into_git_fragments(cmd).any(is_destructive_single)
}

/// Split a bash command string on shell-level separators, returning
/// each fragment that starts with a bare `git` token. Quoted strings
/// and escapes aren't interpreted — sufficient for the guard because
/// we're matching against tokens git itself accepts. Output fragments
/// are trimmed.
fn split_into_git_fragments(cmd: &str) -> impl Iterator<Item = &str> {
    // Shell separators: `&&`, `||`, `;`, `|`. A naive split on any of
    // these characters is correct for the tokens we care about — git
    // arguments never contain them unquoted.
    cmd.split([';', '|', '&']).map(str::trim).filter(|f| !f.is_empty())
}

fn is_destructive_single(fragment: &str) -> bool {
    let mut tokens = fragment.split_whitespace();
    if tokens.next() != Some("git") {
        return false;
    }
    let Some(subcmd) = tokens.next() else {
        return false;
    };
    let rest: Vec<&str> = tokens.collect();

    match subcmd {
        "push" => {
            // `--force-with-lease` is a safe atomic CAS — NOT destructive.
            if rest
                .iter()
                .any(|t| *t == "--force-with-lease" || t.starts_with("--force-with-lease="))
            {
                return false;
            }
            rest.contains(&"--force") || rest.contains(&"-f")
        }
        "reset" => rest.contains(&"--hard"),
        "branch" => rest.contains(&"-D") || rest.contains(&"--delete-force"),
        "checkout" => rest.contains(&"--"),
        "clean" => rest.iter().any(|t| t.starts_with("-f") || *t == "--force"),
        "rebase" => {
            rest.contains(&"--root") || rest.contains(&"-i") || rest.contains(&"--interactive")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_destructive;

    #[test]
    fn detects_force_push() {
        assert!(is_destructive("git push --force"));
        assert!(is_destructive("git push origin main --force"));
        assert!(is_destructive("git push -f origin main"));
    }

    #[test]
    fn exempts_force_with_lease() {
        assert!(!is_destructive("git push --force-with-lease"));
        assert!(!is_destructive("git push --force-with-lease=main origin main"));
        assert!(!is_destructive("git push origin main --force-with-lease"));
    }

    #[test]
    fn detects_reset_hard() {
        assert!(is_destructive("git reset --hard"));
        assert!(is_destructive("git reset --hard HEAD~3"));
        assert!(!is_destructive("git reset HEAD~3"));
        assert!(!is_destructive("git reset --soft HEAD~1"));
    }

    #[test]
    fn detects_branch_force_delete() {
        assert!(is_destructive("git branch -D feature"));
        assert!(!is_destructive("git branch feature"));
        assert!(!is_destructive("git branch -d obsolete"));
    }

    #[test]
    fn detects_rebase_interactive() {
        assert!(is_destructive("git rebase -i HEAD~5"));
        assert!(is_destructive("git rebase --interactive HEAD~5"));
        assert!(is_destructive("git rebase --root"));
        assert!(!is_destructive("git rebase main"));
    }

    #[test]
    fn ignores_non_git() {
        assert!(!is_destructive("ls -la"));
        assert!(!is_destructive("npm install --force"));
        assert!(!is_destructive("rm -rf node_modules"));
    }

    #[test]
    fn catches_destructive_fragment_after_and() {
        // Second-half destructive command must trip the guard.
        assert!(is_destructive("git pull --rebase && git push --force"));
        assert!(is_destructive("git fetch ; git reset --hard origin/main"));
        assert!(is_destructive("git status || git push -f"));
    }

    #[test]
    fn allows_compound_commands_when_every_fragment_is_safe() {
        assert!(!is_destructive("git pull --rebase && git push"));
        assert!(!is_destructive("git fetch ; git status"));
        assert!(!is_destructive("git log | head"));
    }

    #[test]
    fn non_git_fragments_in_compound_are_ignored() {
        // Only the fragment with a leading `git` token is inspected;
        // `rm -rf` in a compound doesn't falsely flag.
        assert!(!is_destructive("rm -rf build/ && git pull"));
        assert!(is_destructive("rm -rf build/ && git push --force"));
    }
}
