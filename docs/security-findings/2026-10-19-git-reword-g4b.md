# Reword of older commits (feature 28, G4b), hacker pass

Scope: `GitHistoryOp::Reword` in `crates/core/src/githistory.rs` (`REWORD_BODY`), loopback test in `crates/core/tests/loopback_sshd.rs`, log sheet entry in `GitLogSheetView.swift`. Live tests ran only in temporary repositories on loopback sshd with a private HOME/tmux socket.

Design: no editor and no `GIT_SEQUENCE_EDITOR`. A new commit is built with `git commit-tree <tree> -p <parent> -m "$4"` (author name, e-mail and date copied from the old commit), then `git rebase --onto <new> <old>` replays the later commits; the tip case uses `reset --soft`. Backup ref `refs/ssh-app/backup/<ts>-reword` before any move; conflicts abort the rebase.

Findings:
1. InputValidation, none: hash is validated (7..40 hex) in Rust and again in the script (`case`, `rev-parse --verify`); the message is passed as `$4`, never interpolated. Tried a message with quotes, backticks, `$(id)`, a leading `-x` and a fake `--- a/x` line: stored verbatim, nothing executed.
2. Rewriting published history: refused when any remote branch contains the commit (tested), merge commits in the range refused, dirty tracked tree refused, merge commit as target refused.
3. Metadata, Low (accepted): the committer is the local git identity, the commit signature (if any) is dropped because the commit is recreated. Stated in the spec.
4. DoS: message capped at 2000 chars, range bounded by git itself; no loop.

Verdict: clean (Low accepted).

## Addendum: SquashRange (arbitrary run of commits)

`GitHistoryOp::SquashRange { oldest, newest, message }`, same technique (`commit-tree` with the tree and author of `newest`, parent of `oldest`, `rebase --onto`). Hashes validated in Rust and in the script; message passed as `$5`. Refusals tested: a single commit, a range starting at a pushed commit (`remote`). Also refused: first commit of the repository, merge commits in or after the range, more than 50 commits, dirty tracked tree, commits outside this branch. Conflicts abort the rebase; backup ref `…-squash` saved. The UI offers it only for an unbroken run of picked commits that does not start at the tip (the tip run keeps the old Squash N button). Verdict: clean.
