# Feature 28: project view for an AI session (files, git status, diff, log, git operations)

Status: spec + phase G0 in progress (2026-10-18). Owner request: one place for the repository the
AI session works in, with git state on the files and, ideally, all of git.

## 1. Goal

Today Files, Diff and Git log are three separate sheets. They become one "Project" view for the
active session's repository (the tmux pane's folder), and the file list shows git state:

- new (green, `N`), modified (orange, `M`), deleted (red, struck through, `D`), renamed, conflicted;
- a folder that contains changes gets a dot of the strongest colour inside it.

Then git operations from the phone, in phases.

## 2. Phases

| Phase | Content | State |
|---|---|---|
| G0 | `fetch_git_status` (core) + coloured file list + Project view with Files / Changes / Log tabs | in progress |
| G1 | Branches: list local/remote, current, ahead/behind; switch; create a branch from a chosen commit or branch | todo |
| G2 | Worktrees: list, add (new branch from X in a new folder), remove; open a session in a worktree | todo |
| G3 | Commit actions: cherry-pick (one or several commits), revert, reset (soft/mixed only), amend message | todo |
| G4 | History rewriting: squash the last N commits (or a selected range), reword, drop; stash; merge/rebase of a branch with conflict reporting | todo |
| G5 | Staging and commit from the phone (stage/unstage files and hunks, commit); push/pull/fetch | todo |

"All of git" is a direction, not a promise: interactive flows that need an editor or a full conflict
resolver (interactive rebase with conflicts, `add -p` edge cases) stay in the terminal; the app shows
what is wrong and offers "open in terminal".

## 3. Safety rules for every operation

- Every operation is a typed core function with validated arguments (refs, hashes, paths) passed as
  separate shell arguments (`$1..`), never spliced into a script; refs checked with
  `git check-ref-format`, hashes as 7..40 hex.
- The target repository is the session pane's repo root, resolved on the host each time; the app
  never sends a free-form path for write operations.
- Read operations run freely. Writing operations need an explicit confirmation sheet that shows the
  exact effect (which commits, which branch). Operations that rewrite history or discard work
  (reset, squash, drop, branch delete, worktree remove) say so in red and are refused when the
  working tree is dirty, unless the user chose "stash first".
- Before any history rewrite the current tip is saved under a ref `refs/ssh-app/backup/<ts>` so
  the previous state can always be restored; the confirmation shows how.
- No force push. Push only fast-forwards unless the user types the branch name to confirm
  `--force-with-lease` (phase G5).
- Hooks and editors: every git call runs with `GIT_EDITOR=true`, `GIT_TERMINAL_PROMPT=0` and
  `core.hooksPath` left as the user set it (commit hooks are the user's own); a call that waits for
  input fails by timeout instead of hanging (host-side watchdog).
- Output is size-capped and made display-safe (control and bidi characters) before it reaches the UI.
- Tests: git operations run in temporary repositories created by the test (never the developer's
  repos); loopback tests use the sshd harness' private HOME.

## 4. G0 design

- Core: `gitstatus` module: one `git status --porcelain=v1 -z` of the repository containing the
  listed folder; paths absolute; capped at 2000 changes / 200 KB; entries with control characters or
  `..` components are dropped. `SshSessionHandle::fetch_git_status(dir)`.
- App: the file browser calls it after every listing, colours entries by state, adds ghost rows for
  deleted files that belong to the listed folder, and marks folders with changes.
- Project view: segmented control Files / Changes / Log in one sheet, reusing the existing diff and
  log views; the toolbar shows the branch.

## 5. Open questions

- Where a worktree should live by default (a sibling folder `<repo>-<branch>`?).
- Whether squash should offer a message editor on the phone (yes, plain text field; no `$EDITOR`).
