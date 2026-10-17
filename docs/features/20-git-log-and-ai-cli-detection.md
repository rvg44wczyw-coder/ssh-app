# 20. Git log and AI CLI detection

## Git log instead of Git diff
The quick-menu tile `diffViewer` now opens **Git log** (`GitLogSheetView`): the latest commits of the
repository the tab's pane is in (hash, subject, author, relative date, refs; "Show more" adds 50).
A tap opens the patch of the commit in the existing diff renderer (`fetch_git_show`; the hash is
validated as 7-40 lowercase hex before it reaches the host script and is passed as an argument).
The working-tree diff is no longer fetched by the app: the buttons `git diff`, `git diff --staged`
and `git status -sb` type the command into the terminal. Rust: `crates/core/src/gitlog.rs`
(`fetch_git_log`, `fetch_git_show`); `fetch_git_diff` stays in the core API but is unused by the UI.

## AI CLI or shell
In Claude Code, Antigravity, Codex and similar CLIs a line is a prompt; a shell command must start
with `!`. `detect_pane_context(session)` (`crates/core/src/pane_context.rs`) reads `#{pane_pid}` from
tmux, lists its descendant processes and recognises known agent programs (also behind
`node`/`python` wrappers and package paths). Result: `Shell`, `AiCli(agent)` or `Unknown` (host error,
treated like a shell). The app caches the answer for 5 s.

`TerminalSessionViewModel.runShellCommand` / `insertShellCommand` add the `!` in an AI CLI
(`shell_command_for_pane`, no double prefix). Used by snippets, AI-assistant suggestions and the
Git log buttons. Agent slash commands (command palette) are not shell commands and are sent as is.

Limits: detection is by process name, so an unknown agent is treated as a shell; `cd` helpers of the
file browser are unchanged (a `!cd` would not change the agent's directory anyway).
