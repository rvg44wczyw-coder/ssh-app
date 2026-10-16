//! Git diff of the repository a terminal tab is working in.
//!
//! An SSH exec channel starts in the user's home directory, where `git diff HEAD` is not a repo
//! command at all (git falls back to `--no-index` and prints its usage text). So the command asks
//! tmux for the pane's current directory and runs git there. The output is parsed strictly: the
//! header lines come before an explicit `GD_DIFF_BEGIN` line, everything after it is diff text.

use crate::error::SshCoreError;
use crate::tmux::{shell_quote, validate_session_name};

pub const MAX_DIFF_BYTES: usize = 1_000_000;
const MAX_UNTRACKED: usize = 50;
const MAX_PATH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitDiffStatus {
    /// A repository was found and `diff` holds its changes (possibly empty = clean).
    Ok,
    /// The terminal's directory is not inside a git repository.
    NotARepo,
    /// The tmux session (or its pane directory) could not be found.
    NoSession,
    /// The host answered with something unexpected.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitDiffResult {
    pub status: GitDiffStatus,
    /// The terminal's working directory (always set when known).
    pub directory: Option<String>,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    /// `git diff HEAD` (tracked changes, staged and unstaged), at most [`MAX_DIFF_BYTES`].
    pub diff: String,
    pub truncated: bool,
    /// New files git does not track yet (they are not part of `diff`).
    pub untracked: Vec<String>,
}

const GIT_DIFF_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
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
git -C "$r" ls-files --others --exclude-standard 2>/dev/null | head -n 50 | while IFS= read -r f; do
  case "$f" in *[![:print:]]*) ;; *) printf 'GD_UNTRACKED=%s\n' "$f";; esac
done
printf 'GD_DIFF_BEGIN\n'
if git -C "$r" rev-parse --verify -q HEAD >/dev/null 2>&1; then
  git -C "$r" -c core.fsmonitor=false diff --no-ext-diff --no-textconv --no-color HEAD 2>&1 | head -c 1000001
else
  # No commit yet: the interesting changes are the staged files.
  git -C "$r" -c core.fsmonitor=false diff --cached --no-ext-diff --no-textconv --no-color 2>&1 | head -c 1000001
fi
exit 0"#;

/// Host command for the diff of the repo the tmux session `session_name` is working in. Runs
/// through the login shell (so `PATH` has git/tmux) under a 20 s host watchdog. The session name
/// is validated and passed as `$1`, never spliced into the script.
pub fn build_git_diff_command(session_name: &str) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    let via_sh = format!(
        "exec sh -c {} sh {}",
        shell_quote(GIT_DIFF_SCRIPT),
        shell_quote(session_name.trim())
    );
    let inner = format!(
        "\"${{SHELL:-/bin/zsh}}\" -l -c {} </dev/null 2>&1",
        shell_quote(&via_sh)
    );
    Ok(crate::hostprobe::watchdog_sh(&inner, 20))
}

/// Display-safe text for a path or name: control characters reject the value, bidi/zero-width
/// characters are shown as visible `⟦U+XXXX⟧` markers.
fn safe_path(s: &str) -> Option<String> {
    (!s.is_empty() && s.len() <= MAX_PATH && !s.chars().any(char::is_control))
        .then(|| crate::approval_queue::display_safe(s))
}

pub fn parse_git_diff_output(output: &str) -> GitDiffResult {
    let (head, diff_raw) = match output.split_once("GD_DIFF_BEGIN\n") {
        Some((h, d)) => (h, Some(d)),
        None => (output, None),
    };
    let mut status = GitDiffStatus::Unknown;
    let mut directory = None;
    let mut repo_root = None;
    let mut branch = None;
    let mut untracked = Vec::new();
    for line in head.lines() {
        if let Some(v) = line.strip_prefix("GD_STATUS=") {
            status = match v {
                "ok" => GitDiffStatus::Ok,
                "not_a_repo" => GitDiffStatus::NotARepo,
                "no_session" => GitDiffStatus::NoSession,
                _ => GitDiffStatus::Unknown,
            };
        } else if let Some(v) = line.strip_prefix("GD_DIR=") {
            directory = safe_path(v);
        } else if let Some(v) = line.strip_prefix("GD_ROOT=") {
            repo_root = safe_path(v);
        } else if let Some(v) = line.strip_prefix("GD_BRANCH=") {
            branch = safe_path(v).filter(|b| b.len() <= 200);
        } else if let Some(v) = line.strip_prefix("GD_UNTRACKED=") {
            if untracked.len() < MAX_UNTRACKED {
                if let Some(p) = safe_path(v) {
                    untracked.push(p);
                }
            }
        }
    }
    let (diff, truncated) = match diff_raw {
        Some(d) if d.len() > MAX_DIFF_BYTES => {
            let mut end = MAX_DIFF_BYTES;
            while !d.is_char_boundary(end) {
                end -= 1;
            }
            (d[..end].to_string(), true)
        }
        Some(d) => (d.to_string(), false),
        None => (String::new(), false),
    };
    if status == GitDiffStatus::Ok && diff_raw.is_none() {
        status = GitDiffStatus::Unknown;
    }
    GitDiffResult {
        status,
        directory,
        repo_root,
        branch,
        diff,
        truncated,
        untracked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_repo_with_changes() {
        let out = "GD_DIR=/w/app\nGD_STATUS=ok\nGD_ROOT=/w/app\nGD_BRANCH=main\nGD_UNTRACKED=new.txt\nGD_DIFF_BEGIN\ndiff --git a/x b/x\n+hi\n";
        let r = parse_git_diff_output(out);
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(
            (
                r.directory.as_deref(),
                r.repo_root.as_deref(),
                r.branch.as_deref()
            ),
            (Some("/w/app"), Some("/w/app"), Some("main"))
        );
        assert_eq!(r.untracked, ["new.txt"]);
        assert_eq!(r.diff, "diff --git a/x b/x\n+hi\n");
        assert!(!r.truncated);
    }

    #[test]
    fn diff_text_cannot_spoof_header_markers() {
        let out = "GD_STATUS=ok\nGD_ROOT=/r\nGD_BRANCH=main\nGD_DIFF_BEGIN\n+GD_ROOT=/evil\nGD_STATUS=not_a_repo\nGD_BRANCH=evil\n";
        let r = parse_git_diff_output(out);
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.repo_root.as_deref(), Some("/r"));
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert!(r.diff.contains("GD_ROOT=/evil"));
    }

    #[test]
    fn not_a_repo_no_session_and_garbage() {
        assert_eq!(
            parse_git_diff_output("GD_DIR=/home/u\nGD_STATUS=not_a_repo\n").status,
            GitDiffStatus::NotARepo
        );
        assert_eq!(
            parse_git_diff_output("GD_STATUS=no_session\n").status,
            GitDiffStatus::NoSession
        );
        // The git usage text that the old command produced is not mistaken for a diff.
        assert_eq!(
            parse_git_diff_output("usage: git diff --no-index <path> <path>\n").status,
            GitDiffStatus::Unknown
        );
        // "ok" without a diff section is not trusted.
        assert_eq!(
            parse_git_diff_output("GD_STATUS=ok\nGD_ROOT=/r\n").status,
            GitDiffStatus::Unknown
        );
    }

    #[test]
    fn caps_the_diff_untracked_list_and_hostile_values() {
        let big = "x".repeat(MAX_DIFF_BYTES + 10);
        let r = parse_git_diff_output(&format!("GD_STATUS=ok\nGD_DIFF_BEGIN\n{big}"));
        assert!(r.truncated && r.diff.len() == MAX_DIFF_BYTES);
        // Multi-byte text is cut on a character boundary.
        let multi = "é".repeat(MAX_DIFF_BYTES);
        let r = parse_git_diff_output(&format!("GD_STATUS=ok\nGD_DIFF_BEGIN\n{multi}"));
        assert!(r.truncated && r.diff.len() <= MAX_DIFF_BYTES);
        let many: String = (0..80).map(|i| format!("GD_UNTRACKED=f{i}\n")).collect();
        let r = parse_git_diff_output(&format!("GD_STATUS=ok\n{many}GD_DIFF_BEGIN\n"));
        assert_eq!(r.untracked.len(), MAX_UNTRACKED);
        let r = parse_git_diff_output(
            "GD_STATUS=ok\nGD_ROOT=/a\u{1b}[31m\nGD_BRANCH=b\u{202e}x\nGD_DIFF_BEGIN\n",
        );
        assert_eq!(r.repo_root, None);
        assert_eq!(r.branch, Some("b⟦U+202E⟧x".to_string()));
    }

    #[test]
    fn command_passes_the_session_name_as_an_argument() {
        let c = build_git_diff_command("claude-1").unwrap();
        assert!(c.starts_with("sh -c "));
        assert!(c.contains("pane_current_path") && c.contains("claude-1"));
        assert!(build_git_diff_command("a b; rm -rf ~").is_err());
        assert!(build_git_diff_command("").is_err());
    }
}
