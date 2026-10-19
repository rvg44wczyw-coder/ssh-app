//! Worktrees of the repository a terminal tab works in (feature 28, phase G2).

use crate::error::SshCoreError;
use crate::file_browser::validate_remote_dir_path;
use crate::gitdiff::GitDiffStatus;
use crate::gitops::{
    op_command, parse_header, parse_op_output, validate_ref_name, validate_start_point,
    GitOpResult, PRELUDE,
};
use crate::tmux::{shell_quote, validate_session_name};

const MAX_WORKTREES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitWorktree {
    pub path: String,
    pub branch: Option<String>,
    pub head_short: String,
    /// The first entry: the repository's own working directory.
    pub is_main: bool,
    /// The worktree this tab's pane is in.
    pub is_current: bool,
    pub detached: bool,
    pub locked: bool,
    /// Its folder is gone; `git worktree prune` would clear the entry.
    pub prunable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitWorktreesResult {
    pub status: GitDiffStatus,
    pub repo_root: Option<String>,
    pub worktrees: Vec<GitWorktree>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitWorktreeOp {
    /// New folder `<repo>-<branch>` next to the repository. `new_branch` creates `branch` at
    /// `from` (current HEAD when none); otherwise `branch` must exist.
    Add {
        branch: String,
        new_branch: bool,
        from: Option<String>,
    },
    /// Removes a worktree listed by git (not the main one, not the current one).
    Remove { path: String, force: bool },
}

const LIST_BODY: &str = r#"printf 'GW_CURRENT=%s\n' "$r"
printf 'GW_BEGIN\n'
g worktree list --porcelain 2>/dev/null | head -c 100000
exit 0"#;

pub fn build_git_worktrees_command(session: &str) -> Result<String, SshCoreError> {
    validate_session_name(session)?;
    let script = format!("{PRELUDE}{LIST_BODY}");
    Ok(crate::hostprobe::login_sh_script(
        &script,
        &[shell_quote(session.trim())],
        20,
    ))
}

pub fn parse_git_worktrees_output(output: &str) -> GitWorktreesResult {
    let (status, repo_root) = parse_header(output);
    let current = output
        .lines()
        .find_map(|l| l.strip_prefix("GW_CURRENT="))
        .map(|s| s.trim().to_string());
    let mut worktrees: Vec<GitWorktree> = vec![];
    if let Some((_, body)) = output.split_once("GW_BEGIN\n") {
        for block in body.split("\n\n") {
            let mut wt = GitWorktree {
                path: String::new(),
                branch: None,
                head_short: String::new(),
                is_main: false,
                is_current: false,
                detached: false,
                locked: false,
                prunable: false,
            };
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    wt.path = p.to_string();
                } else if let Some(h) = line.strip_prefix("HEAD ") {
                    wt.head_short = h.chars().take(7).collect();
                } else if let Some(b) = line.strip_prefix("branch ") {
                    wt.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
                } else if line == "detached" {
                    wt.detached = true;
                } else if line.starts_with("locked") {
                    wt.locked = true;
                } else if line.starts_with("prunable") {
                    wt.prunable = true;
                }
            }
            if wt.path.is_empty()
                || !wt.path.starts_with('/')
                || wt.path.chars().any(char::is_control)
            {
                continue;
            }
            wt.is_main = worktrees.is_empty();
            wt.is_current = current.as_deref() == Some(wt.path.as_str());
            wt.path = crate::approval_queue::display_safe(&wt.path);
            worktrees.push(wt);
            if worktrees.len() >= MAX_WORKTREES {
                break;
            }
        }
    }
    GitWorktreesResult {
        status,
        repo_root,
        worktrees,
    }
}

// $3 = branch, $4 = start point or "", $5 = "new" | "existing".
const ADD_BODY: &str = r#"git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then
  slug=$(printf '%s' "$3" | tr '/' '-')
  dest="$(dirname "$r")/$(basename "$r")-$slug"
  if [ -e "$dest" ]; then rc=2; out="The folder already exists: $dest"
  else
    if [ "$5" = "new" ]; then
      if [ -n "$4" ]; then out=$(g worktree add -b "$3" "$dest" "$4" 2>&1); else out=$(g worktree add -b "$3" "$dest" 2>&1); fi
    else
      out=$(g worktree add "$dest" "$3" 2>&1)
    fi
    rc=$?
    if [ "$rc" = 0 ]; then out="$out
Worktree: $dest"; fi
  fi
fi"#;

// $3 = absolute path, $4 = "force" | "safe".
const REMOVE_BODY: &str = r#"main=$(g worktree list --porcelain 2>/dev/null | sed -n '1s/^worktree //p')
if ! g worktree list --porcelain 2>/dev/null | grep -Fx -- "worktree $3" >/dev/null; then rc=2; out="Not a worktree of this repository"
elif [ "$3" = "$main" ]; then rc=2; out="That is the main working directory"
elif [ "$3" = "$r" ]; then rc=2; out="This tab is inside that worktree; leave it first"
else
  if [ "$4" = "force" ]; then out=$(g worktree remove --force -- "$3" 2>&1); else out=$(g worktree remove -- "$3" 2>&1); fi
  rc=$?
fi"#;

pub fn build_git_worktree_op_command(
    session: &str,
    op: &GitWorktreeOp,
) -> Result<String, SshCoreError> {
    match op {
        GitWorktreeOp::Add {
            branch,
            new_branch,
            from,
        } => {
            let branch = validate_ref_name(branch)?;
            let from = match from {
                Some(f) => validate_start_point(f)?,
                None => "",
            };
            op_command(
                session,
                ADD_BODY,
                &[
                    "add",
                    branch,
                    from,
                    if *new_branch { "new" } else { "existing" },
                ],
            )
        }
        GitWorktreeOp::Remove { path, force } => {
            let path = validate_remote_dir_path(path)?;
            if !path.starts_with('/') || path.split('/').any(|c| c == "..") {
                return Err(crate::agents::invalid("Not an absolute worktree path"));
            }
            op_command(
                session,
                REMOVE_BODY,
                &["remove", &path, if *force { "force" } else { "safe" }],
            )
        }
    }
}

pub fn parse_git_worktree_op_output(output: &str) -> GitOpResult {
    parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_list() {
        let out = "GO_STATUS=ok\nGO_ROOT=/w/app\nGW_CURRENT=/w/app-feat\nGW_BEGIN\n\
worktree /w/app\nHEAD 0123456789abcdef0123456789abcdef01234567\nbranch refs/heads/main\n\n\
worktree /w/app-feat\nHEAD fedcba9876543210fedcba9876543210fedcba98\nbranch refs/heads/feat\nlocked\n\n\
worktree /w/old\nHEAD 1111111111111111111111111111111111111111\ndetached\nprunable gitdir file points to non-existent location\n";
        let r = parse_git_worktrees_output(out);
        assert_eq!(r.worktrees.len(), 3);
        assert!(r.worktrees[0].is_main && !r.worktrees[0].is_current);
        assert_eq!(r.worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(r.worktrees[0].head_short, "0123456");
        assert!(r.worktrees[1].is_current && r.worktrees[1].locked);
        assert!(r.worktrees[2].detached && r.worktrees[2].prunable);
    }

    #[test]
    fn relative_or_odd_paths_are_dropped() {
        let out = "GO_STATUS=ok\nGW_BEGIN\nworktree relative\nHEAD abc\n\nworktree /ok\nHEAD abc\n";
        assert_eq!(parse_git_worktrees_output(out).worktrees.len(), 1);
    }

    #[test]
    fn bad_arguments_build_nothing() {
        let s = "s1";
        let add = |b: &str, from: Option<&str>| {
            build_git_worktree_op_command(
                s,
                &GitWorktreeOp::Add {
                    branch: b.into(),
                    new_branch: true,
                    from: from.map(str::to_string),
                },
            )
        };
        assert!(add("feat/x", None).is_ok());
        assert!(add("-x", None).is_err());
        assert!(add("ok", Some("--orphan")).is_err());
        let rm = |p: &str| {
            build_git_worktree_op_command(
                s,
                &GitWorktreeOp::Remove {
                    path: p.into(),
                    force: false,
                },
            )
        };
        assert!(rm("/w/app-feat").is_ok());
        assert!(rm("relative").is_err());
        assert!(rm("/w/../etc").is_err());
        assert!(rm("/w/a\nb").is_err());
    }
}
