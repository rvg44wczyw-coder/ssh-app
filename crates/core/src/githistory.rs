//! History-changing operations and stashes of the repository a terminal tab works in (feature 28,
//! phase G4): squash the top N commits, drop one commit, merge, rebase, stash push / apply / pop /
//! drop. The rules of `gitcommit` apply (backup ref first, a clean tracked tree, conflicts are
//! aborted by the app), plus: nothing that would rewrite a commit already on a remote branch is
//! ever run, because the app never force-pushes.

use crate::error::SshCoreError;
use crate::gitdiff::GitDiffStatus;
use crate::gitlog::validate_commit_hash;
use crate::gitops::{
    op_command, parse_header, parse_op_output, validate_ref_name, GitOpResult, PRELUDE,
};
use crate::tmux::{shell_quote, validate_session_name};

const MAX_SQUASH: u32 = 50;
const MAX_MESSAGE: usize = 2000;
const MAX_STASHES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GitHistoryOp {
    /// The top `count` commits (2..=50) become one commit with `message`.
    Squash {
        count: u32,
        message: String,
    },
    /// Removes one commit from the branch's history (the later commits are replayed).
    Drop {
        hash: String,
    },
    /// Changes the message of one commit of this branch (the later commits are replayed).
    Reword {
        hash: String,
        message: String,
    },
    /// Merges a branch into the current one (a merge commit when needed).
    Merge {
        branch: String,
    },
    /// Replays the current branch on top of `onto`.
    Rebase {
        onto: String,
    },
    StashPush {
        message: String,
        include_untracked: bool,
    },
    StashApply {
        index: u32,
    },
    StashPop {
        index: u32,
    },
    StashDrop {
        index: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitStash {
    pub index: u32,
    pub message: String,
    pub when: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GitStashesResult {
    pub status: GitDiffStatus,
    pub stashes: Vec<GitStash>,
}

const STASH_LIST_BODY: &str = r#"printf 'GS_BEGIN\n'
g stash list --format='%gd%x1f%gs%x1f%cr%x1e' 2>/dev/null | head -c 50000
exit 0"#;

pub fn build_git_stashes_command(session: &str) -> Result<String, SshCoreError> {
    validate_session_name(session)?;
    let script = format!("{PRELUDE}{STASH_LIST_BODY}");
    Ok(crate::hostprobe::login_sh_script(
        &script,
        &[shell_quote(session.trim())],
        20,
    ))
}

pub fn parse_git_stashes_output(output: &str) -> GitStashesResult {
    let (status, _) = parse_header(output);
    let mut stashes = vec![];
    if let Some((_, body)) = output.split_once("GS_BEGIN\n") {
        for rec in body.split('\u{1e}') {
            let f: Vec<&str> = rec.trim_start_matches('\n').split('\u{1f}').collect();
            if f.len() != 3 {
                continue;
            }
            let Some(index) = f[0]
                .strip_prefix("stash@{")
                .and_then(|r| r.strip_suffix('}'))
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            stashes.push(GitStash {
                index,
                message: crate::approval_queue::display_safe(
                    &f[1].chars().take(200).collect::<String>(),
                ),
                when: crate::approval_queue::display_safe(
                    &f[2].chars().take(40).collect::<String>(),
                ),
            });
            if stashes.len() >= MAX_STASHES {
                break;
            }
        }
    }
    GitStashesResult { status, stashes }
}

// $3 = count, $4 = message.
const SQUASH_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
case "$3" in ""|*[!0-9]*) rc=2; out="Bad commit count";; esac
if [ -z "$rc" ]; then
  n=$3
  total=$(g rev-list --count HEAD 2>/dev/null)
  if [ "$n" -lt 2 ] || [ "$n" -gt 50 ]; then rc=2; out="Squash needs between 2 and 50 commits"
  elif [ -z "$total" ] || [ "$n" -ge "$total" ]; then rc=2; out="The branch needs at least one older commit to squash onto"
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  elif [ -n "$(g rev-list --merges "HEAD~$n..HEAD" 2>/dev/null | head -c 1)" ]; then rc=2; out="A merge commit is among them; squash in the terminal."
  elif [ -n "$(g branch -r --contains "HEAD~$((n-1))" 2>/dev/null | head -c 1)" ]; then rc=2; out="Some of these commits are already on a remote branch; squashing them would need a force push."
  else
    old=$(g rev-parse HEAD)
    bk="refs/ssh-app/backup/$(date +%s)-squash"
    g update-ref "$bk" "$old" 2>/dev/null
    if g reset --soft "HEAD~$n" >/dev/null 2>&1; then
      out=$(g commit -m "$4" 2>&1); rc=$?
      if [ "$rc" != 0 ]; then g reset --soft "$old" >/dev/null 2>&1; g update-ref -d "$bk" 2>/dev/null
        out="$out
The squash failed and the branch was put back."
      else out="$out
Old tip $old saved as $bk (git reset --soft $bk to undo)"; fi
    else rc=1; out="Could not move the branch"; g update-ref -d "$bk" 2>/dev/null; fi
  fi
fi"#;

// $3 = hash.
const DROP_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
case "$3" in ""|*[!0-9a-f]*) rc=2; out="Not a commit";; esac
if [ -z "$rc" ]; then
  if ! g rev-parse --verify -q "$3^{commit}" >/dev/null 2>&1; then rc=2; out="Not a commit of this repository"
  elif ! g merge-base --is-ancestor "$3" HEAD 2>/dev/null; then rc=2; out="That commit is not in this branch's history"
  elif ! g rev-parse --verify -q "$3^" >/dev/null 2>&1; then rc=2; out="That is the first commit of the repository"
  elif [ "$(g rev-list --parents -n 1 "$3" | wc -w | tr -d ' ')" -gt 2 ]; then rc=2; out="That is a merge commit; use the terminal."
  elif [ -n "$(g rev-list --merges "$3..HEAD" 2>/dev/null | head -c 1)" ]; then rc=2; out="A merge commit comes after it; use the terminal."
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  elif [ -n "$(g branch -r --contains "$3" 2>/dev/null | head -c 1)" ]; then rc=2; out="That commit is already on a remote branch; dropping it would need a force push."
  else
    bk="refs/ssh-app/backup/$(date +%s)-drop"
    g update-ref "$bk" HEAD 2>/dev/null
    out=$(g rebase --onto "$3^" "$3" 2>&1); rc=$?
    if [ "$rc" != 0 ]; then
      g rebase --abort >/dev/null 2>&1
      out="$out
Stopped on a conflict. The rebase was aborted; the branch is as before."
    fi
  fi
fi"#;

// $3 = hash, $4 = message. The new commit keeps the tree and the author of the old one.
const REWORD_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
case "$3" in ""|*[!0-9a-f]*) rc=2; out="Not a commit";; esac
if [ -z "$rc" ]; then
  if ! g rev-parse --verify -q "$3^{commit}" >/dev/null 2>&1; then rc=2; out="Not a commit of this repository"
  elif ! g merge-base --is-ancestor "$3" HEAD 2>/dev/null; then rc=2; out="That commit is not in this branch's history"
  elif [ "$(g rev-list --parents -n 1 "$3" | wc -w | tr -d ' ')" -gt 2 ]; then rc=2; out="That is a merge commit; use the terminal."
  elif [ -n "$(g rev-list --merges "$3..HEAD" 2>/dev/null | head -c 1)" ]; then rc=2; out="A merge commit comes after it; use the terminal."
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  elif [ -n "$(g branch -r --contains "$3" 2>/dev/null | head -c 1)" ]; then rc=2; out="That commit is already on a remote branch; rewording it would need a force push."
  else
    old=$(g rev-parse HEAD)
    full=$(g rev-parse "$3^{commit}")
    parent=$(g rev-parse -q --verify "$full^" 2>/dev/null)
    GIT_AUTHOR_NAME=$(g log -1 --format=%an "$full"); export GIT_AUTHOR_NAME
    GIT_AUTHOR_EMAIL=$(g log -1 --format=%ae "$full"); export GIT_AUTHOR_EMAIL
    GIT_AUTHOR_DATE=$(g log -1 --format=%aI "$full"); export GIT_AUTHOR_DATE
    if [ -n "$parent" ]; then new=$(g commit-tree "$full^{tree}" -p "$parent" -m "$4" 2>&1); else new=$(g commit-tree "$full^{tree}" -m "$4" 2>&1); fi
    if [ $? != 0 ]; then rc=1; out="$new"
    else
      bk="refs/ssh-app/backup/$(date +%s)-reword"
      g update-ref "$bk" "$old" 2>/dev/null
      if [ "$full" = "$old" ]; then
        out=$(g reset --soft "$new" 2>&1); rc=$?
      else
        out=$(g rebase --onto "$new" "$full" 2>&1); rc=$?
        if [ "$rc" != 0 ]; then
          g rebase --abort >/dev/null 2>&1
          out="$out
Stopped on a conflict. The rebase was aborted; the branch is as before."
        fi
      fi
      if [ "$rc" = 0 ]; then out="Reworded $full
Old tip $old saved as $bk (git reset --soft $bk to undo)"; fi
    fi
  fi
fi"#;

// $3 = branch.
const MERGE_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then
  if ! g rev-parse --verify -q "$3^{commit}" >/dev/null 2>&1; then rc=2; out="No such branch"
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  else
    bk="refs/ssh-app/backup/$(date +%s)-merge"
    g update-ref "$bk" HEAD 2>/dev/null
    out=$(g merge --no-edit "$3" 2>&1); rc=$?
    if [ "$rc" != 0 ] && gp_exists MERGE_HEAD; then
      g merge --abort >/dev/null 2>&1
      out="$out
Stopped on a conflict. The merge was aborted; the branch is as before."
    fi
  fi
fi"#;

// $3 = onto.
const REBASE_BODY: &str = r#"dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
git check-ref-format --branch "$3" >/dev/null 2>&1 || { rc=2; out="Not a valid branch name"; }
if [ -z "$rc" ]; then
  first=$(g rev-list --reverse "$3..HEAD" 2>/dev/null | head -n 1)
  if ! g rev-parse --verify -q "$3^{commit}" >/dev/null 2>&1; then rc=2; out="No such branch"
  elif [ -z "$(g symbolic-ref -q HEAD 2>/dev/null)" ]; then rc=2; out="HEAD is detached"
  elif [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  elif [ -n "$first" ] && [ -n "$(g branch -r --contains "$first" 2>/dev/null | head -c 1)" ]; then rc=2; out="Commits of this branch are already on a remote branch; rebasing would need a force push."
  else
    bk="refs/ssh-app/backup/$(date +%s)-rebase"
    g update-ref "$bk" HEAD 2>/dev/null
    out=$(g rebase "$3" 2>&1); rc=$?
    if [ "$rc" != 0 ]; then
      g rebase --abort >/dev/null 2>&1
      out="$out
Stopped on a conflict. The rebase was aborted; the branch is as before."
    fi
  fi
fi"#;

// $3 = message, $4 = "untracked" | "tracked".
const STASH_PUSH_BODY: &str = r#"if [ "$4" = "untracked" ]; then out=$(g stash push -u -m "$3" 2>&1); else out=$(g stash push -m "$3" 2>&1); fi
rc=$?"#;

// $3 = index, $4 = apply | pop | drop.
const STASH_USE_BODY: &str = r#"case "$3" in ""|*[!0-9]*) rc=2; out="Bad stash number";; esac
if [ -z "$rc" ]; then
  ref="stash@{$3}"
  sha=$(g rev-parse --verify -q "$ref" 2>/dev/null)
  dirty=$(g status --porcelain -uno 2>/dev/null | head -c 1)
  if [ -z "$sha" ]; then rc=2; out="No such stash"
  elif [ "$4" != drop ] && [ -n "$dirty" ]; then rc=2; out="There are uncommitted changes in tracked files. Commit or stash them first; nothing was changed."
  else
    bk="refs/ssh-app/backup/$(date +%s)-stash"
    g update-ref "$bk" "$sha" 2>/dev/null
    case "$4" in
      apply) out=$(g stash apply "$ref" 2>&1); rc=$?;;
      pop) out=$(g stash apply "$ref" 2>&1); rc=$?
           if [ "$rc" = 0 ]; then g stash drop "$ref" >/dev/null 2>&1; fi;;
      drop) out=$(g stash drop "$ref" 2>&1); rc=$?;;
      *) rc=2; out="Bad action";;
    esac
    if [ "$rc" != 0 ] && [ "$4" != drop ]; then
      g reset --merge >/dev/null 2>&1
      out="$out
The stash could not be applied cleanly; it was left in place and your files are as before."
    fi
  fi
fi"#;

fn clean_message(message: &str, max: usize) -> Result<String, SshCoreError> {
    let m = message.trim();
    if m.is_empty()
        || m.chars().count() > max
        || m.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(crate::agents::invalid("Not a valid message"));
    }
    Ok(m.to_string())
}

pub fn build_git_history_op_command(
    session: &str,
    op: &GitHistoryOp,
) -> Result<String, SshCoreError> {
    match op {
        GitHistoryOp::Squash { count, message } => {
            if !(2..=MAX_SQUASH).contains(count) {
                return Err(crate::agents::invalid(
                    "Squash needs between 2 and 50 commits",
                ));
            }
            op_command(
                session,
                SQUASH_BODY,
                &[
                    "squash",
                    &count.to_string(),
                    &clean_message(message, MAX_MESSAGE)?,
                ],
            )
        }
        GitHistoryOp::Drop { hash } => {
            op_command(session, DROP_BODY, &["drop", validate_commit_hash(hash)?])
        }
        GitHistoryOp::Reword { hash, message } => op_command(
            session,
            REWORD_BODY,
            &[
                "reword",
                validate_commit_hash(hash)?,
                &clean_message(message, MAX_MESSAGE)?,
            ],
        ),
        GitHistoryOp::Merge { branch } => {
            op_command(session, MERGE_BODY, &["merge", validate_ref_name(branch)?])
        }
        GitHistoryOp::Rebase { onto } => {
            op_command(session, REBASE_BODY, &["rebase", validate_ref_name(onto)?])
        }
        GitHistoryOp::StashPush {
            message,
            include_untracked,
        } => op_command(
            session,
            STASH_PUSH_BODY,
            &[
                "stash",
                &clean_message(message, 200)?,
                if *include_untracked {
                    "untracked"
                } else {
                    "tracked"
                },
            ],
        ),
        GitHistoryOp::StashApply { index } => op_command(
            session,
            STASH_USE_BODY,
            &["stash", &index.to_string(), "apply"],
        ),
        GitHistoryOp::StashPop { index } => op_command(
            session,
            STASH_USE_BODY,
            &["stash", &index.to_string(), "pop"],
        ),
        GitHistoryOp::StashDrop { index } => op_command(
            session,
            STASH_USE_BODY,
            &["stash", &index.to_string(), "drop"],
        ),
    }
}

pub fn parse_git_history_op_output(output: &str) -> GitOpResult {
    parse_op_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_checked() {
        let s = "s1";
        let b = |op: GitHistoryOp| build_git_history_op_command(s, &op);
        assert!(b(GitHistoryOp::Squash {
            count: 3,
            message: "one".into()
        })
        .is_ok());
        assert!(b(GitHistoryOp::Squash {
            count: 1,
            message: "one".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Squash {
            count: 51,
            message: "one".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Squash {
            count: 3,
            message: " ".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Reword {
            hash: "abcdef1".into(),
            message: "m".into()
        })
        .is_ok());
        assert!(b(GitHistoryOp::Reword {
            hash: "HEAD".into(),
            message: "m".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Reword {
            hash: "abcdef1".into(),
            message: "".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Drop {
            hash: "HEAD".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Merge {
            branch: "-x".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::Rebase {
            onto: "main;x".into()
        })
        .is_err());
        assert!(b(GitHistoryOp::StashPush {
            message: "wip".into(),
            include_untracked: true
        })
        .is_ok());
        assert!(b(GitHistoryOp::StashPush {
            message: "".into(),
            include_untracked: false
        })
        .is_err());
        assert!(b(GitHistoryOp::StashPop { index: 0 }).is_ok());
    }

    #[test]
    fn parses_stash_list() {
        let out = "GO_STATUS=ok\nGS_BEGIN\nstash@{0}\u{1f}On main: wip\u{1f}2 hours ago\u{1e}\nstash@{1}\u{1f}WIP on x\u{1f}3 days ago\u{1e}\nbad\u{1f}x\u{1f}y\u{1e}";
        let r = parse_git_stashes_output(out);
        assert_eq!(r.stashes.len(), 2);
        assert_eq!(r.stashes[1].index, 1);
        assert_eq!(r.stashes[0].message, "On main: wip");
    }
}
