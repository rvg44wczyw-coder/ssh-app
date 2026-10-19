//! Staging, committing, push / pull / fetch of the repository a terminal tab works in (feature 28,
//! phase G5). Nothing here discards work or rewrites history: no `reset --hard`, no `checkout --`,
//! no force push, `pull` is fast-forward only.

use crate::error::SshCoreError;
use crate::gitdiff::GitDiffStatus;
use crate::gitops::{op_command_secs, parse_header, parse_op_output, GitOpResult, PRELUDE};
use crate::gitstatus::GitFileState;
use crate::tmux::{shell_quote, validate_session_name};

const MAX_ENTRIES: usize = 2000;
const MAX_PATHS: usize = 200;
const MAX_PATH: usize = 4096;
const MAX_MESSAGE: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitChangeEntry {
    /// Relative to the repository root.
    pub path: String,
    /// What is staged for the next commit, if anything.
    pub index_state: Option<GitFileState>,
    /// What changed in the working tree since (or untracked), if anything.
    pub worktree_state: Option<GitFileState>,
    pub orig_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitChangesResult {
    pub status: GitDiffStatus,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<GitChangeEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitIndexOp {
    Stage {
        paths: Vec<String>,
    },
    Unstage {
        paths: Vec<String>,
    },
    StageAll,
    UnstageAll,
    Commit {
        message: String,
    },
    /// Plain `git push` (sets the upstream when there is exactly one remote). Never forces.
    Push,
    /// `git pull --ff-only`.
    Pull,
    /// `git fetch --all`.
    Fetch,
}

const CHANGES_BODY: &str = r#"b=$(g symbolic-ref --short -q HEAD 2>/dev/null)
case "$b" in *[![:print:]]*) b="";; esac
printf 'GX_BRANCH=%s\n' "$b"
u=$(g rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null)
case "$u" in *[![:print:]]*) u="";; esac
printf 'GX_UPSTREAM=%s\n' "$u"
if [ -n "$u" ]; then printf 'GX_COUNTS=%s\n' "$(g rev-list --left-right --count '@{u}...HEAD' 2>/dev/null | tr '\t' ' ')"; fi
printf 'GX_BEGIN\n'
g status --porcelain=v1 -z --untracked-files=normal 2>/dev/null | head -c 300000
exit 0"#;

pub fn build_git_changes_command(session: &str) -> Result<String, SshCoreError> {
    validate_session_name(session)?;
    let script = format!("{PRELUDE}{CHANGES_BODY}");
    Ok(crate::hostprobe::login_sh_script(
        &script,
        &[shell_quote(session.trim())],
        20,
    ))
}

fn state_of(c: u8) -> Option<GitFileState> {
    match c {
        b' ' => None,
        b'?' => Some(GitFileState::Untracked),
        b'A' => Some(GitFileState::Added),
        b'D' => Some(GitFileState::Deleted),
        b'R' | b'C' => Some(GitFileState::Renamed),
        b'U' => Some(GitFileState::Conflicted),
        _ => Some(GitFileState::Modified),
    }
}

fn clean_rel(p: &str) -> Option<String> {
    let t = p.trim_end_matches('/');
    (!t.is_empty()
        && t.len() <= MAX_PATH
        && !t.starts_with('/')
        && !t.split('/').any(|c| c == ".." || c == ".")
        && !t.chars().any(char::is_control))
    .then(|| p.to_string())
}

pub fn parse_git_changes_output(output: &str) -> GitChangesResult {
    let (status, repo_root) = parse_header(output);
    let line = |key: &str| {
        output
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let (ahead, behind) = line("GX_COUNTS=")
        .and_then(|c| {
            let mut it = c.split_whitespace();
            let behind = it.next()?.parse().ok()?;
            let ahead = it.next()?.parse().ok()?;
            Some((ahead, behind))
        })
        .unwrap_or((0, 0));
    let mut entries = vec![];
    let mut truncated = false;
    if let Some((_, body)) = output.split_once("GX_BEGIN\n") {
        let mut fields = body.split('\0');
        while let Some(rec) = fields.next() {
            let b = rec.as_bytes();
            if b.len() < 4 || b[2] != b' ' {
                continue;
            }
            let orig = if matches!(b[0], b'R' | b'C') || matches!(b[1], b'R' | b'C') {
                fields.next().and_then(clean_rel)
            } else {
                None
            };
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let Some(path) = rec.get(3..).and_then(clean_rel) else {
                continue;
            };
            let (index_state, worktree_state) = if (b[0], b[1]) == (b'?', b'?') {
                (None, Some(GitFileState::Untracked))
            } else {
                (state_of(b[0]), state_of(b[1]))
            };
            entries.push(GitChangeEntry {
                path: crate::approval_queue::display_safe(&path),
                index_state,
                worktree_state,
                orig_path: orig,
            });
        }
    }
    GitChangesResult {
        status,
        repo_root,
        branch: line("GX_BRANCH="),
        upstream: line("GX_UPSTREAM="),
        ahead,
        behind,
        entries,
        truncated,
    }
}

// $3.. = paths (after `shift 2`).
const STAGE_BODY: &str = r#"shift 2
GIT_LITERAL_PATHSPECS=1; export GIT_LITERAL_PATHSPECS
out=$(g add -- "$@" 2>&1); rc=$?"#;

const UNSTAGE_BODY: &str = r#"shift 2
GIT_LITERAL_PATHSPECS=1; export GIT_LITERAL_PATHSPECS
if g rev-parse --verify -q HEAD >/dev/null 2>&1; then out=$(g restore --staged -- "$@" 2>&1); else out=$(g rm --cached -r -q -- "$@" 2>&1); fi
rc=$?"#;

const STAGE_ALL_BODY: &str = r#"out=$(g add -A 2>&1); rc=$?"#;

const UNSTAGE_ALL_BODY: &str = r#"if g rev-parse --verify -q HEAD >/dev/null 2>&1; then out=$(g reset -q 2>&1); else out=$(g rm --cached -r -q -- . 2>&1); fi
rc=$?"#;

// $3 = message.
const COMMIT_BODY: &str = r#"if g diff --cached --quiet 2>/dev/null && g rev-parse --verify -q HEAD >/dev/null 2>&1; then rc=2; out="Nothing is staged"
else
  out=$(g commit -m "$3" 2>&1); rc=$?
fi"#;

const PUSH_BODY: &str = r#"GIT_SSH_COMMAND="ssh -o BatchMode=yes"; export GIT_SSH_COMMAND
b=$(g symbolic-ref --short -q HEAD 2>/dev/null)
if [ -z "$b" ]; then rc=2; out="HEAD is detached; there is no branch to push"
elif g rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then out=$(g push 2>&1); rc=$?
else
  remotes=$(g remote 2>/dev/null)
  if [ -z "$remotes" ]; then rc=2; out="This repository has no remote"
  elif [ "$(printf '%s\n' "$remotes" | wc -l | tr -d ' ')" != 1 ]; then rc=2; out="The branch has no upstream and there are several remotes; push once from the terminal (git push -u <remote> $b)."
  else out=$(g push -u "$remotes" "$b" 2>&1); rc=$?; fi
fi"#;

const PULL_BODY: &str = r#"GIT_SSH_COMMAND="ssh -o BatchMode=yes"; export GIT_SSH_COMMAND
dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
if [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
else
  bk="refs/ssh-app/backup/$(date +%s)-pull"
  g update-ref "$bk" HEAD 2>/dev/null
  out=$(g pull --ff-only 2>&1); rc=$?
  if [ "$rc" != 0 ]; then g update-ref -d "$bk" 2>/dev/null; fi
fi"#;

const FETCH_BODY: &str = r#"GIT_SSH_COMMAND="ssh -o BatchMode=yes"; export GIT_SSH_COMMAND
out=$(g fetch --all 2>&1); rc=$?"#;

fn validate_paths(paths: &[String]) -> Result<Vec<&str>, SshCoreError> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(crate::agents::invalid("Pick between 1 and 200 files"));
    }
    paths
        .iter()
        .map(|p| {
            let t = p.as_str();
            if t.is_empty()
                || t.len() > MAX_PATH
                || t.starts_with('-')
                || t.starts_with('/')
                || t.split('/').any(|c| c == "..")
                || t.chars().any(char::is_control)
            {
                Err(crate::agents::invalid("Not a valid repository path"))
            } else {
                Ok(t)
            }
        })
        .collect()
}

pub fn build_git_index_op_command(session: &str, op: &GitIndexOp) -> Result<String, SshCoreError> {
    match op {
        GitIndexOp::Stage { paths } => {
            let mut args = vec!["stage"];
            args.extend(validate_paths(paths)?);
            op_command_secs(session, STAGE_BODY, &args, 30)
        }
        GitIndexOp::Unstage { paths } => {
            let mut args = vec!["unstage"];
            args.extend(validate_paths(paths)?);
            op_command_secs(session, UNSTAGE_BODY, &args, 30)
        }
        GitIndexOp::StageAll => op_command_secs(session, STAGE_ALL_BODY, &["stage_all"], 30),
        GitIndexOp::UnstageAll => op_command_secs(session, UNSTAGE_ALL_BODY, &["unstage_all"], 30),
        GitIndexOp::Commit { message } => {
            let m = message.trim();
            if m.is_empty()
                || m.chars().count() > MAX_MESSAGE
                || m.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(crate::agents::invalid("Not a valid commit message"));
            }
            op_command_secs(session, COMMIT_BODY, &["commit", m], 60)
        }
        GitIndexOp::Push => op_command_secs(session, PUSH_BODY, &["push"], 60),
        GitIndexOp::Pull => op_command_secs(session, PULL_BODY, &["pull"], 60),
        GitIndexOp::Fetch => op_command_secs(session, FETCH_BODY, &["fetch"], 60),
    }
}

pub fn parse_git_index_op_output(output: &str) -> GitOpResult {
    parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_index_and_worktree_state() {
        let out = format!(
            "GO_STATUS=ok\nGO_ROOT=/r\nGX_BRANCH=main\nGX_UPSTREAM=origin/main\nGX_COUNTS=1 2\nGX_BEGIN\n{}",
            ["M  staged.rs", " M edited.rs", "MM both.rs", "A  new.rs", "?? u.txt", "R  b.rs", "a.rs", " D gone.rs", "UU c.rs"]
                .iter()
                .map(|r| format!("{r}\0"))
                .collect::<String>()
        );
        let r = parse_git_changes_output(&out);
        assert_eq!((r.ahead, r.behind), (2, 1));
        assert_eq!(r.upstream.as_deref(), Some("origin/main"));
        let e = |p: &str| r.entries.iter().find(|e| e.path == p).unwrap();
        assert_eq!(e("staged.rs").index_state, Some(GitFileState::Modified));
        assert_eq!(e("staged.rs").worktree_state, None);
        assert_eq!(e("edited.rs").index_state, None);
        assert_eq!(e("edited.rs").worktree_state, Some(GitFileState::Modified));
        assert!(e("both.rs").index_state.is_some() && e("both.rs").worktree_state.is_some());
        assert_eq!(e("new.rs").index_state, Some(GitFileState::Added));
        assert_eq!(e("u.txt").worktree_state, Some(GitFileState::Untracked));
        assert_eq!(e("b.rs").orig_path.as_deref(), Some("a.rs"));
        assert_eq!(e("gone.rs").worktree_state, Some(GitFileState::Deleted));
        assert_eq!(e("c.rs").index_state, Some(GitFileState::Conflicted));
        assert_eq!(
            r.entries.len(),
            8,
            "the rename source is not a separate entry"
        );
    }

    #[test]
    fn arguments_are_checked() {
        let s = "s1";
        let stage = |p: &[&str]| {
            build_git_index_op_command(
                s,
                &GitIndexOp::Stage {
                    paths: p.iter().map(|x| x.to_string()).collect(),
                },
            )
        };
        assert!(stage(&["src/a.rs", "dir with space/b.txt"]).is_ok());
        assert!(stage(&[]).is_err());
        assert!(stage(&["-A"]).is_err());
        assert!(stage(&["../x"]).is_err());
        assert!(stage(&["/etc/passwd"]).is_err());
        assert!(stage(&["a\nb"]).is_err());
        let commit =
            |m: &str| build_git_index_op_command(s, &GitIndexOp::Commit { message: m.into() });
        assert!(commit("fix: x\n\nbody").is_ok());
        assert!(commit("  ").is_err());
        assert!(commit("a\u{0}b").is_err());
    }
}
