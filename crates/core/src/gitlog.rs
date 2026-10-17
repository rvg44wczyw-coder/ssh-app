//! Git history of the repository a terminal tab is working in, and the patch of one commit.
//!
//! Same approach as `gitdiff`: the host command asks tmux for the pane's directory and runs git
//! there, header lines come before an explicit marker and everything after it is git's output.
//! The working-tree diff itself is no longer fetched by the app; it is run as a command in the
//! terminal (see `pane_context` for the `!` prefix inside an AI CLI).

use crate::error::SshCoreError;
use crate::gitdiff::{GitDiffResult, GitDiffStatus};
use crate::tmux::{shell_quote, validate_session_name};

const MAX_COMMITS: u32 = 200;
const MAX_LOG_BYTES: usize = 400_000;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitLogEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    /// "3 hours ago".
    pub when: String,
    /// Branch and tag names pointing at the commit (`HEAD -> main, origin/main`), may be empty.
    pub refs: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitLogResult {
    pub status: GitDiffStatus,
    pub directory: Option<String>,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub entries: Vec<GitLogEntry>,
    /// The history is longer than what was asked for.
    pub has_more: bool,
}

const GIT_LOG_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
p=$(tmux display-message -p -t "=$1:" '#{pane_current_path}' 2>/dev/null)
if [ -z "$p" ]; then printf 'GD_STATUS=no_session\n'; exit 0; fi
case "$p" in *[![:print:]]*) printf 'GD_STATUS=no_session\n'; exit 0;; esac
printf 'GD_DIR=%s\n' "$p"
r=$(git -C "$p" rev-parse --show-toplevel 2>/dev/null)
case "$r" in ""|*[![:print:]]*) printf 'GD_STATUS=not_a_repo\n'; exit 0;; esac
printf 'GD_STATUS=ok\nGD_ROOT=%s\n' "$r"
b=$(git -C "$r" rev-parse --abbrev-ref HEAD 2>/dev/null)
case "$b" in *[![:print:]]*) b="";; esac
printf 'GD_BRANCH=%s\n' "$b"
printf 'GD_DIFF_BEGIN\n'
git -C "$r" -c core.fsmonitor=false log -n "$2" --no-color --no-ext-diff --date=relative --decorate=short --pretty=format:'%H%x1f%h%x1f%an%x1f%ar%x1f%D%x1f%s%x1e' 2>/dev/null | head -c 400000
exit 0"#;

const GIT_SHOW_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
p=$(tmux display-message -p -t "=$1:" '#{pane_current_path}' 2>/dev/null)
if [ -z "$p" ]; then printf 'GD_STATUS=no_session\n'; exit 0; fi
case "$p" in *[![:print:]]*) printf 'GD_STATUS=no_session\n'; exit 0;; esac
printf 'GD_DIR=%s\n' "$p"
r=$(git -C "$p" rev-parse --show-toplevel 2>/dev/null)
case "$r" in ""|*[![:print:]]*) printf 'GD_STATUS=not_a_repo\n'; exit 0;; esac
printf 'GD_STATUS=ok\nGD_ROOT=%s\n' "$r"
printf 'GD_BRANCH=%s\n' "$2"
printf 'GD_DIFF_BEGIN\n'
git -C "$r" -c core.fsmonitor=false show --no-color --no-ext-diff --no-textconv --format=fuller --stat --patch "$2" -- 2>&1 | head -c 1000001
exit 0"#;

/// A commit id as typed by git: 7 to 40 lowercase hex digits. Anything else never reaches git.
pub fn validate_commit_hash(hash: &str) -> Result<&str, SshCoreError> {
    let h = hash.trim();
    if (7..=40).contains(&h.len())
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(h)
    } else {
        Err(crate::agents::invalid("Not a commit id"))
    }
}

pub fn build_git_log_command(session_name: &str, limit: u32) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    // One more than shown, so "has more" is known.
    let n = limit.clamp(1, MAX_COMMITS) + 1;
    Ok(crate::hostprobe::login_sh_script(
        GIT_LOG_SCRIPT,
        &[shell_quote(session_name.trim()), n.to_string()],
        20,
    ))
}

pub fn build_git_show_command(session_name: &str, hash: &str) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    let hash = validate_commit_hash(hash)?;
    Ok(crate::hostprobe::login_sh_script(
        GIT_SHOW_SCRIPT,
        &[shell_quote(session_name.trim()), shell_quote(hash)],
        20,
    ))
}

fn clean(s: &str, max: usize) -> String {
    crate::approval_queue::display_safe(&s.chars().take(max).collect::<String>())
}

pub fn parse_git_log_output(output: &str, limit: u32) -> GitLogResult {
    // The header parser of the diff is reused: same markers, the "diff" part is the log text.
    let head = crate::gitdiff::parse_git_diff_output(output);
    let limit = limit.clamp(1, MAX_COMMITS) as usize;
    let mut entries = vec![];
    if let Some((_, log)) = output.split_once("GD_DIFF_BEGIN\n") {
        let log = if log.len() > MAX_LOG_BYTES {
            let mut end = MAX_LOG_BYTES;
            while !log.is_char_boundary(end) {
                end -= 1;
            }
            &log[..end]
        } else {
            log
        };
        for rec in log.split('\u{1e}') {
            let rec = rec.trim_start_matches('\n');
            let f: Vec<&str> = rec.split('\u{1f}').collect();
            if f.len() != 6 {
                continue;
            }
            let hash = f[0].trim();
            if validate_commit_hash(hash).is_err() || hash.len() != 40 {
                continue;
            }
            entries.push(GitLogEntry {
                hash: hash.to_string(),
                short_hash: clean(f[1], 12),
                author: clean(f[2], 80),
                when: clean(f[3], 40),
                refs: clean(f[4], 200),
                subject: clean(f[5], 300),
            });
        }
    }
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    GitLogResult {
        status: head.status,
        directory: head.directory,
        repo_root: head.repo_root,
        branch: head.branch,
        entries,
        has_more,
    }
}

/// `git show` of one commit, in the shape the diff viewer already renders.
pub fn parse_git_show_output(output: &str) -> GitDiffResult {
    crate::gitdiff::parse_git_diff_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "0123456789abcdef0123456789abcdef01234567";
    const H2: &str = "fedcba9876543210fedcba9876543210fedcba98";

    fn rec(h: &str, subject: &str) -> String {
        format!(
            "{h}\u{1f}{}\u{1f}Ann\u{1f}2 hours ago\u{1f}HEAD -> main\u{1f}{subject}\u{1e}",
            &h[..7]
        )
    }

    #[test]
    fn parses_commits_and_has_more() {
        let out = format!(
            "GD_DIR=/w\nGD_STATUS=ok\nGD_ROOT=/w\nGD_BRANCH=main\nGD_DIFF_BEGIN\n{}\n{}",
            rec(H1, "feat: one"),
            rec(H2, "fix: two")
        );
        let r = parse_git_log_output(&out, 1);
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.entries.len(), 1);
        assert!(r.has_more);
        assert_eq!(r.entries[0].subject, "feat: one");
        assert_eq!(r.entries[0].short_hash, "0123456");
        assert_eq!(r.branch.as_deref(), Some("main"));
        let all = parse_git_log_output(&out, 5);
        assert_eq!(all.entries.len(), 2);
        assert!(!all.has_more);
    }

    #[test]
    fn hostile_log_text_cannot_forge_commits_or_markers() {
        // A subject cannot contain the separators git emits (git strips them from the format
        // placeholders only by chance), so records with a wrong shape or hash are dropped.
        let bad = format!(
            "zz\u{1f}a\u{1f}b\u{1f}c\u{1f}d\u{1f}e\u{1e}short\u{1f}x\u{1e}{}",
            rec(H1, "ok\u{7}\u{202e}evil")
        );
        let out = format!("GD_STATUS=ok\nGD_ROOT=/w\nGD_DIFF_BEGIN\n{bad}");
        let r = parse_git_log_output(&out, 10);
        assert_eq!(r.entries.len(), 1);
        assert!(!r.entries[0].subject.contains('\u{202e}'));
        assert!(
            r.entries[0].subject.contains("⟦U+202E⟧") || !r.entries[0].subject.contains('\u{7}')
        );
    }

    #[test]
    fn not_a_repo_and_no_session() {
        assert_eq!(
            parse_git_log_output("GD_DIR=/h\nGD_STATUS=not_a_repo\n", 5).status,
            GitDiffStatus::NotARepo
        );
        assert_eq!(
            parse_git_log_output("GD_STATUS=no_session\n", 5).status,
            GitDiffStatus::NoSession
        );
        assert!(parse_git_log_output("GD_STATUS=no_session\n", 5)
            .entries
            .is_empty());
    }

    #[test]
    fn commit_ids_are_checked_before_they_reach_git() {
        assert!(validate_commit_hash(H1).is_ok());
        assert!(validate_commit_hash("abc1234").is_ok());
        for bad in [
            "",
            "abc",
            "--output=/tmp/x",
            "HEAD",
            "ABCDEF1234",
            "abc123; rm -rf /",
            "g123456",
            &"a".repeat(41),
        ] {
            assert!(validate_commit_hash(bad).is_err(), "{bad:?}");
        }
        assert!(build_git_show_command("term-1", "--exec=evil").is_err());
        let cmd = build_git_show_command("term-1", H1).unwrap();
        assert!(cmd.contains(H1));
        assert!(build_git_log_command("bad name!", 10).is_err());
        assert!(build_git_log_command("term-1", 10).is_ok());
    }
}
