//! Branches of the repository a terminal tab works in: list, switch, create, delete
//! (feature 28, phase G1). See `gitops` for the shared safety rules.

use crate::error::SshCoreError;
use crate::gitdiff::GitDiffStatus;
use crate::gitops::{
    op_command, parse_header, parse_op_output, validate_ref_name, validate_start_point,
    GitOpResult, PRELUDE,
};
use crate::tmux::{shell_quote, validate_session_name};

const MAX_BRANCHES: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitBranch {
    /// `main`, or `origin/main` for a remote branch.
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub short_hash: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitBranchesResult {
    pub status: GitDiffStatus,
    pub repo_root: Option<String>,
    pub current: Option<String>,
    /// HEAD points at a commit, not at a branch.
    pub detached: bool,
    /// Tracked files have uncommitted changes.
    pub dirty: bool,
    pub branches: Vec<GitBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitBranchOp {
    /// `git switch <name>` (a local branch).
    Switch { name: String },
    /// `git switch --track <origin/name>`: a local branch that follows the remote one.
    SwitchTracking { name: String },
    /// New branch at `from` (current HEAD when none); `switch_to` also checks it out.
    Create {
        name: String,
        from: Option<String>,
        switch_to: bool,
    },
    /// `git branch -d` (merged only) or `-D`; the tip is saved under `refs/ssh-app/backup/` first.
    Delete { name: String, force: bool },
}

const LIST_BODY: &str = r#"cur=$(g symbolic-ref --short -q HEAD 2>/dev/null)
case "$cur" in *[![:print:]]*) cur="";; esac
printf 'GB_CURRENT=%s\n' "$cur"
if [ -n "$(g status --porcelain -uno 2>/dev/null | head -c 1)" ]; then printf 'GB_DIRTY=1\n'; else printf 'GB_DIRTY=0\n'; fi
printf 'GB_BEGIN\n'
g for-each-ref --format='%(refname)%1f%(objectname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(subject)%1e' refs/heads refs/remotes 2>/dev/null | head -c 100000
exit 0"#;

pub fn build_git_branches_command(session: &str) -> Result<String, SshCoreError> {
    validate_session_name(session)?;
    let script = format!("{PRELUDE}{LIST_BODY}");
    Ok(crate::hostprobe::login_sh_script(
        &script,
        &[shell_quote(session.trim())],
        20,
    ))
}

fn parse_track(track: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

fn clean(s: &str, max: usize) -> String {
    crate::approval_queue::display_safe(&s.chars().take(max).collect::<String>())
}

pub fn parse_git_branches_output(output: &str) -> GitBranchesResult {
    let (status, repo_root) = parse_header(output);
    let line = |key: &str| {
        output
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
    };
    let current = line("GB_CURRENT=").filter(|c| !c.is_empty());
    let dirty = line("GB_DIRTY=").as_deref() == Some("1");
    let mut branches = vec![];
    if let Some((_, body)) = output.split_once("GB_BEGIN\n") {
        for rec in body.split('\u{1e}') {
            let f: Vec<&str> = rec.trim_start_matches('\n').split('\u{1f}').collect();
            if f.len() != 5 {
                continue;
            }
            let (name, is_remote) = if let Some(n) = f[0].strip_prefix("refs/heads/") {
                (n, false)
            } else if let Some(n) = f[0].strip_prefix("refs/remotes/") {
                (n, true)
            } else {
                continue;
            };
            if name.ends_with("/HEAD") || validate_ref_name(name).is_err() {
                continue;
            }
            let (ahead, behind) = parse_track(f[3]);
            branches.push(GitBranch {
                name: name.to_string(),
                is_remote,
                is_current: !is_remote && current.as_deref() == Some(name),
                upstream: Some(f[2].trim().to_string()).filter(|u| !u.is_empty()),
                ahead,
                behind,
                short_hash: clean(f[1], 12),
                subject: clean(f[4], 200),
            });
            if branches.len() >= MAX_BRANCHES {
                break;
            }
        }
    }
    let detached = status == GitDiffStatus::Ok && current.is_none();
    GitBranchesResult {
        status,
        repo_root,
        current,
        detached,
        dirty,
        branches,
    }
}

// Each body reads its arguments from $3 (and $4); $2 is unused here (kept for the shared layout).
const SWITCH_BODY: &str = r#"git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then out=$(g switch "$3" 2>&1); rc=$?; fi"#;

const SWITCH_TRACKING_BODY: &str = r#"git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then out=$(g switch --track "$3" 2>&1); rc=$?; fi"#;

const CREATE_BODY: &str = r#"git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then
  if [ "$5" = "switch" ]; then
    if [ -n "$4" ]; then out=$(g switch -c "$3" "$4" 2>&1); else out=$(g switch -c "$3" 2>&1); fi
  else
    if [ -n "$4" ]; then out=$(g branch "$3" "$4" 2>&1); else out=$(g branch "$3" 2>&1); fi
  fi
  rc=$?
fi"#;

const DELETE_BODY: &str = r#"git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then
  cur=$(g symbolic-ref --short -q HEAD 2>/dev/null)
  tip=$(g rev-parse --verify -q "refs/heads/$3^{commit}" 2>/dev/null)
  if [ "$cur" = "$3" ]; then rc=2; out="Cannot delete the branch you are on"
  elif [ -z "$tip" ]; then rc=2; out="No such local branch"
  else
    bk="refs/ssh-app/backup/$(date +%s)-$3"
    g update-ref "$bk" "$tip" 2>/dev/null
    if [ "$4" = "force" ]; then out=$(g branch -D -- "$3" 2>&1); else out=$(g branch -d -- "$3" 2>&1); fi
    rc=$?
    if [ "$rc" = 0 ]; then out="$out
Saved as $bk ($tip)"; else g update-ref -d "$bk" 2>/dev/null; fi
  fi
fi"#;

pub fn build_git_branch_op_command(
    session: &str,
    op: &GitBranchOp,
) -> Result<String, SshCoreError> {
    match op {
        GitBranchOp::Switch { name } => {
            op_command(session, SWITCH_BODY, &["switch", validate_ref_name(name)?])
        }
        GitBranchOp::SwitchTracking { name } => op_command(
            session,
            SWITCH_TRACKING_BODY,
            &["track", validate_ref_name(name)?],
        ),
        GitBranchOp::Create {
            name,
            from,
            switch_to,
        } => {
            let name = validate_ref_name(name)?;
            let from = match from {
                Some(f) => validate_start_point(f)?,
                None => "",
            };
            op_command(
                session,
                CREATE_BODY,
                &[
                    "create",
                    name,
                    from,
                    if *switch_to { "switch" } else { "keep" },
                ],
            )
        }
        GitBranchOp::Delete { name, force } => op_command(
            session,
            DELETE_BODY,
            &[
                "delete",
                validate_ref_name(name)?,
                if *force { "force" } else { "safe" },
            ],
        ),
    }
}

pub fn parse_git_branch_op_output(output: &str) -> GitOpResult {
    parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(refname: &str, up: &str, track: &str) -> String {
        format!("{refname}\u{1f}abc1234\u{1f}{up}\u{1f}{track}\u{1f}subject\u{1e}")
    }

    #[test]
    fn parses_branches_with_tracking_state() {
        let out = format!(
            "GO_STATUS=ok\nGO_ROOT=/r\nGB_CURRENT=main\nGB_DIRTY=1\nGB_BEGIN\n{}\n{}\n{}\n{}",
            rec("refs/heads/main", "origin/main", "ahead 2, behind 1"),
            rec("refs/heads/feat/x", "", ""),
            rec("refs/remotes/origin/main", "", ""),
            rec("refs/remotes/origin/HEAD", "", ""),
        );
        let r = parse_git_branches_output(&out);
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.current.as_deref(), Some("main"));
        assert!(r.dirty && !r.detached);
        assert_eq!(r.branches.len(), 3, "origin/HEAD is not a branch");
        let main = &r.branches[0];
        assert!(main.is_current && !main.is_remote);
        assert_eq!((main.ahead, main.behind), (2, 1));
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert!(r.branches[2].is_remote && !r.branches[2].is_current);
    }

    #[test]
    fn detached_head_and_foreign_refs() {
        let out = format!(
            "GO_STATUS=ok\nGB_CURRENT=\nGB_DIRTY=0\nGB_BEGIN\n{}\n{}",
            rec("refs/heads/a", "", ""),
            rec("refs/tags/v1", "", "")
        );
        let r = parse_git_branches_output(&out);
        assert!(r.detached);
        assert_eq!(r.branches.len(), 1);
    }

    #[test]
    fn bad_input_never_builds_a_command() {
        let s = "sess";
        assert!(
            build_git_branch_op_command(s, &GitBranchOp::Switch { name: "-f".into() }).is_err()
        );
        assert!(build_git_branch_op_command(
            s,
            &GitBranchOp::Create {
                name: "ok".into(),
                from: Some("--orphan".into()),
                switch_to: true
            }
        )
        .is_err());
        assert!(build_git_branch_op_command(
            s,
            &GitBranchOp::Delete {
                name: "a;b".into(),
                force: true
            }
        )
        .is_err());
        assert!(build_git_branch_op_command(
            s,
            &GitBranchOp::Switch {
                name: "main".into()
            }
        )
        .is_ok());
    }

    #[test]
    fn track_text() {
        assert_eq!(parse_track("ahead 3"), (3, 0));
        assert_eq!(parse_track("behind 4"), (0, 4));
        assert_eq!(parse_track("gone"), (0, 0));
    }
}
