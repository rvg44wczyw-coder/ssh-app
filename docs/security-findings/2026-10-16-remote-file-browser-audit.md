# Security Audit — M13 Remote File Browser & Viewer (2026-10-16)

## 1. Scope
- `crates/core/src/file_browser.rs`, `SshSessionHandle::list_remote_directory` / `read_remote_file_preview` (`session.rs`), `FileBrowserSheetView.swift`, file-browser parts of `TerminalSessionViewModel.swift`.
- Applicable: input validation, path traversal / symlinks, command injection, DoS, terminal keystroke injection, metadata leakage. Ruled out: crypto, MITM/replay/downgrade (no new protocol; reuses the authenticated SSH session).
- Live tests (local temp tree, `sh`/`ls`/`realpath`/`mkfifo`, no network): symlink file → `~/.ssh/id_rsa`, symlink dir → `.ssh`, FIFO, filename with `\r`, filename `$(touch PWNED)'; id`. Rust-side behaviour verified via unit tests (53 passing; `file_browser.rs` coverage 96.6%).

## 2. Findings (all fixed before this report; re-verified)
1. **PathTraversal — Medium (fixed)**: lexical blocklist bypassed by symlink (`notes.txt -> ~/.ssh/id_rsa`; `innocent -> ~/.ssh`). Live: `realpath` resolves both. Fix: remote `realpath` first, re-validate resolved path (`build_resolve_path_command`, `session.rs`); listing re-validates `pwd -P` result.
2. **InputValidation — Medium (fixed)**: file names containing control chars (`\r`, ESC, ^C) shown by `ls` raw (confirmed live `a^Mb`) and would be typed into the terminal by "Insert Path"/"cd Here" (keystroke injection). Fix: entries and cwd with control chars are dropped (`parse_ls_line`, tested).
3. **DoS — Low (fixed)**: preview of a FIFO/device blocks the exec channel forever. Fix: `[ -f ]` guard, regular files only (verified live: FIFO refused).
4. **InputValidation — Low (fixed)**: blocklist missed `.env.*`, `.npmrc`, `.pgpass`, `.pypirc`, `.git-credentials`. Fix: `.env` prefix match + extended list.
5. **Injection — none**: hostile name `$(touch PWNED)'; id` is single-quote escaped; live test shows no execution. 
6. **Residual — Low (accepted)**: unbounded `ls -la` output for huge directories; blocklist is a best-effort denylist (arbitrary secrets in other file names are still readable by design — the user owns the host and the SSH session already grants full shell).

## 3. Verdict
Findings (highest severity: Medium), all remediated; no open issues.
