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
| G1 | Branches: list local/remote, current, ahead/behind; switch; create a branch from a chosen commit or branch | done (core + sheet; device check pending) |
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

## 4a. G1 design: branches (2026-10-19)

Owner decisions (2026-10-19, "делаем все"): defaults accepted - worktrees go to a sibling folder
`<repo>-<branch>`, squash message is a plain text field.

Core module `gitbranch`:

- `fetch_git_branches(session)` -> `GitBranchesResult { status, repo_root, current, detached,
  dirty, branches[] }`; a branch is `{ name, is_remote, is_current, upstream, ahead, behind,
  short_hash, subject }` (one `git for-each-ref` over `refs/heads` and `refs/remotes`, capped at
  300 entries).
- `run_git_branch_op(session, op)` with `op` = `Switch { name }` (a remote name becomes a tracking
  branch), `Create { name, from, switch }`, `Delete { name, force }` -> `GitOpResult { status,
  ok, exit_code, output }`.
- Validation in the core (before anything is sent) and again on the host with
  `git check-ref-format --branch`: no leading `-`, no whitespace / control / `~^:?*[\`, no `..`,
  no `@{`, no trailing `/` `.` or `.lock`, at most 200 bytes. `from` is such a name or 7..40 hex.
  Everything is passed as `$2..`, the operation as a fixed keyword picked by an allow-list.
- Deleting a branch first saves its tip under `refs/ssh-app/backup/<epoch>-<name>`; `-D` only with
  `force`. Deleting the current branch is refused by git and by the app. Switching never forces
  (git refuses when local changes would be overwritten and says so in `output`).
- Runs in the pane's repository, `GIT_EDITOR=true GIT_TERMINAL_PROMPT=0`, 20 s watchdog, output
  capped at 20 KB and display-safe.

App: a "Branches" sheet from the branch label in the file browser: list (local first, current
marked, ahead/behind), tap = switch (confirmation names the effect and warns when the tree is
dirty), "+" = new branch from the current one or from a tapped row, swipe = delete (red, force is
a second confirmation with the lost commit count unknown -> says "may lose commits").

Tests: validators and parsers (unit); a temporary repository on the loopback host: list, create,
switch, delete (backup ref exists), refusal of bad names and of deleting the current branch.

## 5. Open questions

- Resolved 2026-10-19: worktrees in a sibling folder `<repo>-<branch>`; squash message in a plain text field (no `$EDITOR`).
