//! PostToolUse(git commit) → append `MilestoneShipped { Verified }`.

use knotch_agent::{
    HookInput, HookOutput,
    active::{ActiveUnit, project_root, resolve_active_for_hook},
    causation::hook_causation,
};
use knotch_kernel::CommitRef;
use knotch_workflow::ConfigWorkflow;

use crate::config::Config;

pub(crate) async fn run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput> {
    let Some(command) = input.bash_command() else {
        return Ok(HookOutput::Continue);
    };
    if !command.trim_start().starts_with("git commit") {
        return Ok(HookOutput::Continue);
    }
    let Some(msg) = knotch_agent::commit::extract_commit_message(command) else {
        return Ok(HookOutput::Continue);
    };
    let Some(sha) = input.bash_response_stdout().and_then(extract_sha_from_stdout) else {
        return Ok(HookOutput::Continue);
    };
    let root = project_root(&input.cwd);
    let unit = match resolve_active_for_hook(&root, input.session_id.as_str())? {
        ActiveUnit::Active(u) => u,
        _ => return Ok(HookOutput::Continue),
    };
    let causation = hook_causation(&input, "verify-commit");
    let commit_ref = CommitRef::new(compact_str::CompactString::from(sha));
    let repo = config.build_repository()?;
    Ok(knotch_agent::commit::verify::<ConfigWorkflow, _>(&repo, &unit, &msg, commit_ref, causation)
        .await?)
}

/// Extract the commit SHA from `git commit` stdout.
///
/// Git emits a header like `[<branch> <sha>] <subject>` where
/// `<branch>` can be multi-token (`detached HEAD`, `root-commit
/// main`, etc.) and `<sha>` is always a lone 7+ hex token inside
/// the brackets. Some variants add `(amend)` / `(root-commit)`
/// suffixes after the SHA. When `color.ui = always` is set the
/// header is wrapped in ANSI colour escapes, which we strip before
/// the scan.
///
/// The walk scans every `[...]` header and picks the first
/// bracket-internal token that looks like a SHA (≥7 hex chars).
fn extract_sha_from_stdout(stdout: &str) -> Option<String> {
    let cleaned = strip_ansi_escapes(stdout);
    for line in cleaned.lines() {
        let trimmed = line.trim_start();
        let Some(stripped) = trimmed.strip_prefix('[') else {
            continue;
        };
        let Some(end) = stripped.find(']') else {
            continue;
        };
        let header = &stripped[..end];
        for raw in header.split_whitespace() {
            // Strip surrounding parens from suffix tokens like
            // `(amend)` / `(root-commit)`.
            let token = raw.trim_matches(|c| c == '(' || c == ')');
            if token.len() >= 7 && token.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(token.to_owned());
            }
        }
    }
    None
}

/// Remove CSI-style ANSI escape sequences (`ESC [ … <letter>`).
/// Covers the colour / cursor / erase escapes git emits under
/// `color.ui = always`. We walk the byte stream in a single pass
/// so it is O(n) with no allocation beyond the output string.
fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume the `[`
            // Consume the parameter bytes (digits, `;`, `?`) until the
            // terminating letter (`@`..`~` final byte — colour ends on
            // `m`, others can too).
            while let Some(&next) = chars.peek() {
                chars.next();
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::extract_sha_from_stdout;

    #[test]
    fn extracts_standard_commit_header() {
        assert_eq!(
            extract_sha_from_stdout("[main abc1234] feat: add login\n").as_deref(),
            Some("abc1234")
        );
    }

    #[test]
    fn extracts_detached_head_header() {
        assert_eq!(
            extract_sha_from_stdout("[detached HEAD 0f1e2d3] fix: typo\n").as_deref(),
            Some("0f1e2d3")
        );
    }

    #[test]
    fn extracts_amend_header() {
        assert_eq!(
            extract_sha_from_stdout("[main 9876abc (amend)] fix: typo\n").as_deref(),
            Some("9876abc")
        );
    }

    #[test]
    fn extracts_root_commit_header() {
        assert_eq!(
            extract_sha_from_stdout("[main (root-commit) deadbee] initial\n").as_deref(),
            Some("deadbee")
        );
    }

    #[test]
    fn ignores_lines_without_bracket_header() {
        assert!(extract_sha_from_stdout("create mode 100644 src/main.rs\n").is_none());
    }

    #[test]
    fn extracts_sha_from_ansi_coloured_header() {
        // `git -c color.ui=always commit ...` wraps tokens in CSI
        // escape sequences. Strip them before the header scan.
        let coloured = "\x1b[32m[\x1b[33mmain\x1b[0m \x1b[1;36mabc1234\x1b[0m] feat: add login\n";
        assert_eq!(extract_sha_from_stdout(coloured).as_deref(), Some("abc1234"));
    }

    #[test]
    fn ansi_stripper_is_noop_for_plain_input() {
        // Regression — the ANSI pass must not eat normal brackets
        // or spaces.
        assert_eq!(extract_sha_from_stdout("[main abc1234] subject\n").as_deref(), Some("abc1234"));
    }
}
