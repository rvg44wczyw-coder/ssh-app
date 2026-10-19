//! Operations on commits of the repository a terminal tab works in (feature 28, phase G3):
//! cherry-pick, revert, reset (soft / mixed only) and reword of the last commit. Every operation
//! saves the old tip under `refs/ssh-app/backup/<epoch>-<what>` first, refuses a dirty tree where
//! git would mix local changes into the result, and undoes a conflicted pick or revert itself, so
//! a failure never leaves the repository half-way through.

use crate::error::SshCoreError;
use crate::gitlog::validate_commit_hash;
use crate::gitops::{op_command, GitOpResult};

const MAX_PICKS: usize = 20;
const MAX_MESSAGE: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GitResetMode {
    /// Moves the branch; index and files stay as they are (the undone commits show as staged).
    Soft,
    /// Moves the branch and unstages; files stay as they are.
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitCommitOp {
    /// Applies the commits in the given order (oldest first) on top of HEAD.
    CherryPick { hashes: Vec<String> },
    /// A new commit that undoes `hash` (not for merge commits).
    Revert { hash: String },
    /// Moves the current branch to an earlier commit of its own history. No `--hard`: the app
    /// never throws away working files.
    Reset { hash: String, mode: GitResetMode },
    /// Rewrites the message of the last commit; refused once that commit exists on a remote branch.
    AmendMessage { message: String },
}

// $3 = space separated 7..40 hex ids.
const CHERRY_PICK_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
bad=""
for h in $3; do
  case "$h" in ""|*[!0-9a-f]*) bad=1;; esac
  g rev-parse --verify -q "$h^{commit}" >/dev/null 2>&1 || bad=1
done
if [ -n "$bad" ]; then rc=2; out="Not a commit of this repository"
elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
else
  bk="refs/ssh-app/backup/$(date +%s)-cherry-pick"
  g update-ref "$bk" HEAD 2>/dev/null
  out=$(g cherry-pick $3 2>&1); rc=$?
  if [ "$rc" != 0 ]; then
    if gp_exists CHERRY_PICK_HEAD || gp_exists sequencer; then
      g cherry-pick --abort >/dev/null 2>&1
      out="$out
Stopped (conflict or empty commit). The cherry-pick was aborted; the branch is as before. Resolve it in the terminal if you want it."
    fi
  fi
fi"#;

// $3 = hash.
const REVERT_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
case "$3" in ""|*[!0-9a-f]*) rc=2; out="Not a commit";; esac
if [ -z "$rc" ]; then
  parents=$(g rev-list --parents -n 1 "$3" 2>/dev/null | wc -w | tr -d ' ')
  if [ -z "$parents" ] || [ "$parents" = 0 ]; then rc=2; out="Not a commit of this repository"
  elif [ "$parents" -gt 2 ]; then rc=2; out="That is a merge commit; revert it in the terminal (git revert -m 1 ...)"
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  else
    bk="refs/ssh-app/backup/$(date +%s)-revert"
    g update-ref "$bk" HEAD 2>/dev/null
    out=$(g revert --no-edit "$3" 2>&1); rc=$?
    if [ "$rc" != 0 ] && gp_exists REVERT_HEAD; then
      g revert --abort >/dev/null 2>&1
      out="$out
Stopped on a conflict. The revert was aborted; the branch is as before."
    fi
  fi
fi"#;

// $3 = hash, $4 = soft | mixed.
const RESET_BODY: &str = r#"case "$3" in ""|*[!0-9a-f]*) rc=2; out="Not a commit";; esac
if [ -z "$rc" ]; then
  if ! g rev-parse --verify -q "$3^{commit}" >/dev/null 2>&1; then rc=2; out="Not a commit of this repository"
  elif ! g merge-base --is-ancestor "$3" HEAD 2>/dev/null; then rc=2; out="That commit is not in this branch's history; use the terminal for that."
  else
    old=$(g rev-parse HEAD 2>/dev/null)
    bk="refs/ssh-app/backup/$(date +%s)-reset"
    g update-ref "$bk" "$old" 2>/dev/null
    if [ "$4" = "soft" ]; then out=$(g reset --soft "$3" 2>&1); else out=$(g reset --mixed "$3" 2>&1); fi
    rc=$?
    if [ "$rc" = 0 ]; then out="$out
Previous tip $old saved as $bk (git reset --soft $bk to undo)"; else g update-ref -d "$bk" 2>/dev/null; fi
  fi
fi"#;

// $3 = message.
const AMEND_BODY: &str = r#"if [ -z "$(g rev-parse --verify -q HEAD 2>/dev/null)" ]; then rc=2; out="There is no commit yet"
elif [ -n "$(g branch -r --contains HEAD 2>/dev/null | head -c 1)" ]; then rc=2; out="The last commit is already on a remote branch; rewriting it would need a force push."
else
  bk="refs/ssh-app/backup/$(date +%s)-amend"
  g update-ref "$bk" HEAD 2>/dev/null
  out=$(g commit --amend --only -m "$3" 2>&1); rc=$?
  if [ "$rc" != 0 ]; then g update-ref -d "$bk" 2>/dev/null; else out="$out
Previous commit saved as $bk"; fi
fi"#;

fn clean_message(message: &str) -> Result<String, SshCoreError> {
    let m = message.trim();
    if m.is_empty()
        || m.chars().count() > MAX_MESSAGE
        || m.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(crate::agents::invalid("Not a valid commit message"));
    }
    Ok(m.to_string())
}

pub fn build_git_commit_op_command(
    session: &str,
    op: &GitCommitOp,
) -> Result<String, SshCoreError> {
    match op {
        GitCommitOp::CherryPick { hashes } => {
            if hashes.is_empty() || hashes.len() > MAX_PICKS {
                return Err(crate::agents::invalid("Pick between 1 and 20 commits"));
            }
            let valid: Result<Vec<&str>, _> =
                hashes.iter().map(|h| validate_commit_hash(h)).collect();
            op_command(session, CHERRY_PICK_BODY, &["pick", &valid?.join(" ")])
        }
        GitCommitOp::Revert { hash } => op_command(
            session,
            REVERT_BODY,
            &["revert", validate_commit_hash(hash)?],
        ),
        GitCommitOp::Reset { hash, mode } => op_command(
            session,
            RESET_BODY,
            &[
                "reset",
                validate_commit_hash(hash)?,
                match mode {
                    GitResetMode::Soft => "soft",
                    GitResetMode::Mixed => "mixed",
                },
            ],
        ),
        GitCommitOp::AmendMessage { message } => {
            op_command(session, AMEND_BODY, &["amend", &clean_message(message)?])
        }
    }
}

pub fn parse_git_commit_op_output(output: &str) -> GitOpResult {
    crate::gitops::parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn arguments_are_checked_before_anything_is_built() {
        let s = "s1";
        let pick = |hs: Vec<&str>| {
            build_git_commit_op_command(
                s,
                &GitCommitOp::CherryPick {
                    hashes: hs.into_iter().map(str::to_string).collect(),
                },
            )
        };
        assert!(pick(vec![H, "abcdef1"]).is_ok());
        assert!(pick(vec![]).is_err());
        assert!(pick(vec!["HEAD~1"]).is_err());
        assert!(pick(vec![H, "abc; rm -rf /"]).is_err());
        assert!(pick(vec!["abcdef1"; 21]).is_err());
        let reset = |h: &str| {
            build_git_commit_op_command(
                s,
                &GitCommitOp::Reset {
                    hash: h.into(),
                    mode: GitResetMode::Soft,
                },
            )
        };
        assert!(reset(H).is_ok());
        assert!(reset("--hard").is_err());
        let amend = |m: &str| {
            build_git_commit_op_command(s, &GitCommitOp::AmendMessage { message: m.into() })
        };
        assert!(amend("fix: thing\n\nbody").is_ok());
        assert!(amend("   ").is_err());
        assert!(amend("a\u{0}b").is_err());
        assert!(amend(&"x".repeat(2001)).is_err());
        assert!(
            build_git_commit_op_command(s, &GitCommitOp::Revert { hash: "zz".into() }).is_err()
        );
    }
}
