//! Git-commit event translation.
//!
//! Three hook entry points:
//!
//! - [`check`] — `PreToolUse(git commit *)`. Validates that the proposed milestone (if
//!   the commit carries a trailer) is not already in the unit's shipped set. Emits no
//!   events.
//! - [`verify_proposal`] — `PostToolUse(git commit *)`. Produces the `MilestoneShipped {
//!   status: Verified }` proposal when the commit's `Knotch-Milestone:` trailer names a
//!   milestone. The CLI layer passes it to [`crate::queue::post_tool_append`] so the
//!   append runs under the retry + queue + orphan contract
//!   (`.claude/rules/hook-integration.md`).
//! - [`revert_proposal`] — `PostToolUse(git revert *)`. Produces the `MilestoneReverted`
//!   proposal tying the revert SHA to the original.
//!
//! ## Milestone opt-in
//!
//! A commit becomes a milestone event **only** when its message
//! carries an explicit `Knotch-Milestone: <id>` git trailer. One
//! feature typically lands across several incremental commits
//! ("start X", "polish X", "test X"); a slug-from-description
//! heuristic would inflate the log with distinct events per commit
//! and defeat dedup. The trailer forces the author to **name** a
//! milestone exactly once, on whichever commit finalizes it.
//!
//! See `docs/cli-schemas/README.md` and
//! `.claude/rules/hook-integration.md` for the operator-facing
//! description.

use knotch_kernel::{
    Causation, CommitKind, CommitRef, CommitStatus, EventBody, MilestoneKind, Proposal, Repository,
    UnitId, WorkflowKind, project::shipped_milestones,
};

use crate::{error::HookError, output::HookOutput};

/// `PreToolUse(git commit)` entry.
///
/// Blocks when the commit's `Knotch-Milestone:` trailer names a
/// milestone already present in the unit's shipped set. Non-trailer
/// commits always pass — they are not milestones by policy.
pub async fn check<W, R>(
    repo: &R,
    unit: &UnitId,
    commit_message: &str,
) -> Result<HookOutput, HookError>
where
    W: WorkflowKind,
    R: Repository<W>,
{
    let Some(id) = extract_milestone_id(commit_message) else {
        return Ok(HookOutput::Continue);
    };
    let Some(milestone) = repo.workflow().parse_milestone(&id) else {
        return Ok(HookOutput::block(format!(
            "workflow `{}` rejected milestone id `{id}`",
            repo.workflow().name()
        )));
    };
    let log = repo.load(unit).await?;
    let new_id = milestone.id();
    if shipped_milestones(&log).iter().any(|m| m.id() == new_id) {
        return Ok(HookOutput::block(format!(
            "milestone `{id}` already shipped in unit `{}` — pick a different id \
             or supersede the prior event",
            unit.as_str()
        )));
    }
    Ok(HookOutput::Continue)
}

/// Build a `MilestoneShipped { Verified }` proposal from a
/// `PostToolUse(git commit)` invocation.
///
/// Returns `None` when the commit carries no `Knotch-Milestone:`
/// trailer, when the workflow rejects the milestone id, or when the
/// commit message lacks a recognized Conventional-Commits prefix
/// (`MilestoneShipped` requires a valid `CommitKind`). In every
/// `None` case the hook falls back to [`HookOutput::Continue`] at
/// the CLI boundary — a non-milestone commit is a silent no-op by
/// design.
///
/// The CLI layer wraps the returned proposal with the retry + queue
/// + orphan contract in [`crate::queue::post_tool_append`].
#[must_use]
pub fn verify_proposal<W>(
    workflow: &W,
    commit_message: &str,
    commit_sha: CommitRef,
    causation: Causation,
) -> Option<Proposal<W>>
where
    W: WorkflowKind,
    W::Extension: Default,
{
    let id = extract_milestone_id(commit_message)?;
    let Some(milestone) = workflow.parse_milestone(&id) else {
        let workflow_name = workflow.name().into_owned();
        tracing::warn!(workflow = %workflow_name, id, "verify: workflow rejected milestone id");
        return None;
    };
    let (kind, _) = parse_conventional(commit_message)?;
    Some(Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::MilestoneShipped {
            milestone,
            commit: commit_sha,
            commit_kind: kind,
            status: CommitStatus::Verified,
        },
        supersedes: None,
    })
}

/// Build a `MilestoneReverted` proposal from a
/// `PostToolUse(git revert)` invocation. The caller resolves the
/// `milestone` back-reference by scanning the unit's log for the
/// matching prior `MilestoneShipped`; passing a mismatched milestone
/// produces a proposal whose kernel precondition will reject on
/// append.
#[must_use]
pub fn revert_proposal<W>(
    revert: CommitRef,
    original: CommitRef,
    milestone: W::Milestone,
    causation: Causation,
) -> Proposal<W>
where
    W: WorkflowKind,
    W::Extension: Default,
{
    Proposal {
        causation,
        extension: <W::Extension as Default>::default(),
        body: EventBody::MilestoneReverted { milestone, original, revert },
        supersedes: None,
    }
}

/// Parse the first line of a conventional-commit message into
/// `(kind, description)`. Used by [`verify_proposal`] to populate
/// `MilestoneShipped::commit_kind` — the kernel precondition
/// rejects non-implementation kinds.
///
/// Returns `None` when the first line isn't a recognized
/// Conventional-Commits prefix.
#[must_use]
pub fn parse_conventional(msg: &str) -> Option<(CommitKind, String)> {
    let first_line = msg.lines().next()?;
    let (head, rest) = first_line.split_once(':')?;
    let kind_str = head.split_once('(').map_or(head, |(k, _)| k).trim();
    let kind = match kind_str {
        "feat" => CommitKind::Feat,
        "fix" => CommitKind::Fix,
        "refactor" => CommitKind::Refactor,
        "perf" => CommitKind::Perf,
        "docs" => CommitKind::Docs,
        "chore" => CommitKind::Chore,
        "test" => CommitKind::Test,
        "ci" => CommitKind::Ci,
        "build" => CommitKind::Build,
        "style" => CommitKind::Style,
        "revert" => CommitKind::Revert,
        _ => return None,
    };
    let description = rest.trim().to_owned();
    if description.is_empty() {
        return None;
    }
    Some((kind, description))
}

/// Extract the `Knotch-Milestone: <id>` trailer from a commit
/// message. See module docs for the opt-in rationale.
///
/// Accepts any case spelling of the key. Git's own trailer parser
/// is key-case-sensitive by default, but upstream tooling commonly
/// normalises to lowercase; we match case-insensitively so an agent
/// that spells the trailer `knotch-milestone:` doesn't silently
/// drop the milestone on the floor. The canonical spelling in docs
/// stays `Knotch-Milestone:`.
#[must_use]
pub fn extract_milestone_id(commit_message: &str) -> Option<String> {
    const KEY: &str = "knotch-milestone:";
    for line in commit_message.lines() {
        let trimmed = line.trim_start();
        if trimmed.len() < KEY.len() {
            continue;
        }
        let (prefix, rest) = trimmed.split_at(KEY.len());
        if prefix.eq_ignore_ascii_case(KEY) {
            let id = rest.trim();
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}

/// Reassemble the effective commit message from a `git commit`
/// command line, handling all the ways Git accepts a message:
///
/// - `-m <msg>` / `--message=<msg>` — repeatable; multiple values join with a blank-line
///   separator (git convention).
/// - `-F <path>` / `--file=<path>` — read message body from disk. `-F -` (stdin) is not
///   supported; callers cannot replay stdin.
///
/// Shell quoting / escaping goes through [`shell_words::split`]
/// which applies **POSIX shell** rules — `\` is an escape character,
/// `"..."` and `'...'` have their POSIX semantics. That means this
/// function is Unix-only: on Windows the `bash_command` that Claude
/// Code hands us comes from cmd.exe / PowerShell, whose quoting
/// rules are incompatible with POSIX, so any attempt to parse it
/// here produces garbage tokens.
///
/// To keep the contract narrow rather than fragile, the function is
/// compiled only on Unix targets. On Windows the stub returns `None`
/// so PostToolUse hooks silently no-op for `git commit` until a
/// platform-native parser is wired in (follow-up when a Windows
/// adopter asks for milestone recording — `SubprocessObserver`
/// itself is already cross-platform, this gap is commit-hook
/// specific).
///
/// Returns `None` when the command is not `git commit`, when no
/// message source is present, or when the command fails to tokenize.
#[cfg(unix)]
#[must_use]
pub fn extract_message(cmd: &str) -> Option<String> {
    let tokens = shell_words::split(cmd).ok()?;
    let mut it = tokens.iter();

    if it.next().map(String::as_str) != Some("git") {
        return None;
    }
    if it.next().map(String::as_str) != Some("commit") {
        return None;
    }

    let mut messages: Vec<String> = Vec::new();
    let mut file_path: Option<String> = None;

    while let Some(tok) = it.next() {
        match tok.as_str() {
            "-m" | "--message" => {
                if let Some(value) = it.next() {
                    messages.push(value.clone());
                }
            }
            s if s.starts_with("--message=") => {
                messages.push(s["--message=".len()..].to_owned());
            }
            "-F" | "--file" => {
                if let Some(path) = it.next() {
                    file_path = Some(path.clone());
                }
            }
            s if s.starts_with("--file=") => {
                file_path = Some(s["--file=".len()..].to_owned());
            }
            _ => {}
        }
    }

    // `-F <file>` takes precedence per git semantics (the last
    // message source wins, but we only support one of each kind).
    if let Some(path) = file_path {
        if path == "-" {
            // stdin is not replayable from a stored command string
            return None;
        }
        if let Ok(body) = std::fs::read_to_string(&path) {
            return Some(body);
        }
        return None;
    }

    if messages.is_empty() {
        return None;
    }
    // Git joins multiple `-m` values with a blank line as separator.
    Some(messages.join("\n\n"))
}

/// Windows stub — POSIX-shell quoting is not portable to cmd.exe /
/// PowerShell. Returning `None` keeps the PostToolUse commit hook
/// as a silent no-op on Windows rather than mangling a command
/// string through the wrong shell grammar.
#[cfg(not(unix))]
#[must_use]
pub fn extract_message(_cmd: &str) -> Option<String> {
    None
}

// Platform-independent parser tests — run on every CI target.
#[cfg(test)]
mod parser_tests {
    use super::*;

    // ---- parse_conventional ----

    #[test]
    fn parse_conventional_handles_scope() {
        let (kind, desc) = parse_conventional("feat(auth): add sso").unwrap();
        assert_eq!(kind, CommitKind::Feat);
        assert_eq!(desc, "add sso");
    }

    #[test]
    fn parse_conventional_skips_unknown_prefix() {
        assert!(parse_conventional("wip: scratch").is_none());
        assert!(parse_conventional("plain message").is_none());
    }

    // ---- extract_milestone_id ----

    #[test]
    fn milestone_id_trailer_single_line() {
        let msg = "feat: add login\n\nKnotch-Milestone: SPEC-123";
        assert_eq!(extract_milestone_id(msg).as_deref(), Some("SPEC-123"));
    }

    #[test]
    fn milestone_id_empty_trailer_ignored() {
        let msg = "feat: polish\n\nKnotch-Milestone:";
        assert!(extract_milestone_id(msg).is_none());
    }

    #[test]
    fn milestone_id_absent_without_trailer() {
        assert!(extract_milestone_id("feat: add login").is_none());
        assert!(extract_milestone_id("plain commit").is_none());
    }

    #[test]
    fn milestone_id_trailer_is_case_insensitive() {
        for key in ["Knotch-Milestone:", "knotch-milestone:", "KNOTCH-MILESTONE:"] {
            let msg = format!("feat: login\n\n{key} SPEC-7");
            assert_eq!(
                extract_milestone_id(&msg).as_deref(),
                Some("SPEC-7"),
                "case variant `{key}` should match",
            );
        }
    }
}

// `extract_message` is POSIX-only (see its `#[cfg(unix)]`).
// On Windows the stub returns `None` for every input, so the tests
// below would either be no-ops (asserting `Some(...)`) or false-green
// (asserting `None`). Gate the whole suite on Unix instead.
#[cfg(all(test, unix))]
mod commit_message_tests {
    use super::*;

    // ---- extract_message ----

    #[test]
    fn commit_msg_single_m_double_quote() {
        let cmd = r#"git commit -m "feat: add login""#;
        assert_eq!(extract_message(cmd).as_deref(), Some("feat: add login"));
    }

    #[test]
    fn commit_msg_single_m_single_quote() {
        let cmd = "git commit -m 'feat: add login'";
        assert_eq!(extract_message(cmd).as_deref(), Some("feat: add login"));
    }

    #[test]
    fn commit_msg_message_equals_flag() {
        let cmd = r#"git commit --message="feat: add login""#;
        assert_eq!(extract_message(cmd).as_deref(), Some("feat: add login"));
    }

    #[test]
    fn commit_msg_multiple_m_joined_with_blank_line() {
        // Git convention: second `-m` is a body paragraph.
        let cmd = r#"git commit -m "feat: add login" -m "Knotch-Milestone: SPEC-123""#;
        assert_eq!(
            extract_message(cmd).as_deref(),
            Some("feat: add login\n\nKnotch-Milestone: SPEC-123")
        );
        // And the trailer extractor picks the second paragraph up:
        let body = extract_message(cmd).unwrap();
        assert_eq!(extract_milestone_id(&body).as_deref(), Some("SPEC-123"));
    }

    #[test]
    fn commit_msg_m_and_long_form_mixed() {
        let cmd = r#"git commit --message="feat: subject" -m "Knotch-Milestone: X""#;
        let body = extract_message(cmd).unwrap();
        assert_eq!(extract_milestone_id(&body).as_deref(), Some("X"));
    }

    #[test]
    fn commit_msg_file_flag_reads_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("msg.txt");
        std::fs::write(&path, "feat: from file\n\nKnotch-Milestone: FROMFILE").unwrap();
        let cmd = format!("git commit -F {}", path.display());
        let body = extract_message(&cmd).unwrap();
        assert_eq!(extract_milestone_id(&body).as_deref(), Some("FROMFILE"));
    }

    #[test]
    fn commit_msg_stdin_is_unsupported() {
        // `git commit -F -` reads from stdin — we cannot replay it
        // from the stored command alone, so we refuse rather than
        // silently misinterpret.
        assert!(extract_message("git commit -F -").is_none());
    }

    #[test]
    fn commit_msg_returns_none_for_non_git() {
        assert!(extract_message("ls -la").is_none());
        assert!(extract_message("echo hello").is_none());
    }

    #[test]
    fn commit_msg_returns_none_without_message_source() {
        // `git commit` with no -m / -F — opens $EDITOR, not
        // replayable here.
        assert!(extract_message("git commit --amend").is_none());
    }
}
