# Git branches (feature 28, G1) — security pass

Scope: `gitops.rs` (validators, prelude), `gitbranch.rs` (scripts, parser), `GitBranchesSheetView`.
Live: the loopback test `git_branch_operations_run_in_a_temporary_repo_on_loopback_sshd` ran the
real scripts against a temporary repository through the test sshd (private tmux, private HOME);
real tmux pid and HOME unchanged afterwards.

1. **Option / shell injection through names (InputValidation)** — branch and start-point names go
   through `validate_ref_name` (no leading `-`, no whitespace, control or shell metacharacters,
   no `..`, `@{`, `.lock`) and are passed as `$3`/`$4` of a fixed script; the operation is a fixed
   keyword chosen by a Rust enum. Host side repeats `git check-ref-format --branch`. Tested with
   `-f`, `a;b`, `x y`, `a..b` (refused before any host call). No finding.
2. **Data loss (Medium if unmitigated)** — branch delete saves the tip under
   `refs/ssh-app/backup/<ts>-<name>` first; a refused delete removes that backup again; `-D` only
   after a second confirmation. Deleting the current branch refused (script and git). Verified live.
3. **Hostile repository config (SandboxEscape)** — every call runs git with `core.fsmonitor=false`,
   `log.showSignature=false`, `gpg.program=false`; `GIT_EDITOR=true`, `GIT_TERMINAL_PROMPT=0`.
   Repository hooks (post-checkout) are the user's own and do run on `switch`, as in the terminal;
   stated in the spec, accepted.
4. **Output (MetadataLeak/XSS-like)** — names and subjects pass `display_safe` and length caps;
   remote refs failing validation are dropped from the list.
5. **Resource use (DoS)** — 300 branches, 100 KB list output, 20 KB op output, 20–25 s watchdog.

Verdict: clean. Unverified on device: the sheet itself.

## Addendum: worktrees (G2)

- `Remove` takes an absolute path without `..`/control characters, and the host script removes it
  only when git's own `worktree list --porcelain` lists exactly that path, it is not the main
  worktree and not the one the tab is in. Verified live: the main worktree, an unrelated folder
  and relative / `..` paths were refused; the unrelated folder still existed afterwards.
- `Add` computes the destination on the host from the validated branch name (`/` becomes `-`) next
  to the repository, and refuses an existing folder. Verified live (second add of the same name).
- Without `force`, git refuses to remove a dirty worktree (verified); `force` needs a second
  confirmation naming the folder.
- A tab's start folder (new `TabSession.startDir`) goes through the existing `validate_start_dir`.

Verdict: clean.

## Addendum: commit and history operations (G3, G4)

Live (temporary repositories on the loopback sshd, private tmux/HOME): cherry-pick (one, several,
conflicting, unknown id), revert (dirty tree refused, normal), reset soft/mixed (ancestor only),
reword (staged files not swept in; refused once on a remote), squash (pushed base refused,
unpushed top N squashed, message kept), drop (conflict aborted, pushed refused), merge and rebase
(conflict aborted, tree clean, HEAD unchanged), stash push/pop/drop (dirty tree blocks pop,
untracked included on request). Real tmux pid and HOME unchanged after every run.

1. **Data loss (would be High without mitigations)** — every operation writes
   `refs/ssh-app/backup/<epoch>-<what>` first (removed again if the operation is refused after the
   fact); no `--hard`, no force, no force-push anywhere in the app. Verified: backup refs exist.
2. **Rewriting published history (Medium)** — squash, drop, rebase and reword refuse when the
   affected commits are contained in any remote branch (`git branch -r --contains`). Verified.
3. **Half-finished operations (Medium)** — a conflicting cherry-pick, revert, merge, rebase or
   drop is aborted by the script (`--abort`), so the repository is never left mid-operation;
   `gp_exists` resolves git's state files relative to the repository (a first version looked in
   the wrong directory and left a conflict behind; caught by the loopback test, fixed).
4. **Injection (InputValidation)** — hashes (hex only, checked in Rust and again in the script),
   branch names (`validate_ref_name` + `git check-ref-format`), messages (control characters
   refused, 2000 chars) and numbers are passed as `$n`; the only unquoted expansion is
   `for h in $3` over space-joined validated hex ids. Refused-before-the-host cases are unit tested.
5. **Hooks (SandboxEscape, accepted)** — commit/merge/rebase run the repository's own hooks like
   the terminal does; `core.fsmonitor`, `gpg.program` and signature display are disabled.

Verdict: clean (no Medium or higher open).

## Addendum: staging, commit, remote operations (G5)

Live (temporary repo plus a bare "remote" on the loopback host; no network): stage / unstage /
stage-all / unstage-all, commit (refused with nothing staged), push without upstream (one remote →
`-u`), push rejected as non-fast-forward (not forced), fetch, pull (dirty tracked tree refused,
diverged → `--ff-only` refuses and changes nothing, clean fast-forward works), several remotes
and no upstream → refused.

1. **Pathspec injection (InputValidation)** — paths are repository-relative, no `..`, no leading
   `-` or `/`, no control characters, at most 200, passed after `--` with
   `GIT_LITERAL_PATHSPECS=1`. Verified: a file literally named `star*.txt` is staged, `star1.txt`
   is not (no glob expansion); `-A`, `../x`, `/etc/passwd`, newline paths are refused before the
   host is contacted.
2. **Force / destructive (High if present)** — none exists: no `--force`, no `--hard`, no discard
   of working changes; `pull` is `--ff-only`; the old tip is saved before a pull.
3. **Credential prompts / hangs (DoS)** — `GIT_TERMINAL_PROMPT=0` and
   `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` make a missing credential fail fast; 60 s watchdog
   kills the process tree.
4. **Hooks (SandboxEscape, accepted)** — commit and push run the repository's own hooks as the
   terminal would; `fsmonitor` and `gpg.program` are disabled.
5. **Remote choice (Spoofing-like)** — with several remotes and no upstream the app refuses to
   pick one for the user.

Verdict: clean.
