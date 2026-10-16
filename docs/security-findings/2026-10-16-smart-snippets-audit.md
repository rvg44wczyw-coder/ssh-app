# Security Audit Report — Feature 08: Smart Snippets & Fuzzy History Search (Milestone M11)

**Date**: 2026-10-16  
**Auditor**: `hacker` skill  
**Target**:
- `crates/core/src/snippets.rs`
- `crates/core/src/session.rs` (`fetch_remote_shell_history`)
- `ios/Sources/SSHApp/Views/SnippetsSheetView.swift`
- `ios/Sources/SSHApp/ViewModels/TerminalSessionViewModel.swift`

---

## 1. Threat Model & Attack Surface

1. **Shell History Secret Exposure**:
   - Developer shell history (`~/.zsh_history`, `~/.bash_history`) frequently contains accidentally pasted API keys (`OPENAI_API_KEY=...`, `ANTHROPIC_API_KEY=...`), bearer tokens (`Authorization: Bearer ...`), or passwords.
   - Surfacing raw shell history in the UI or storing it in local snippet caches risks leaking credentials.
2. **Command Injection via History Fetching or Snippet Execution**:
   - Fetching remote shell history executes a remote command over SSH.
   - Snippet placeholder substitution (`{{branch}}`, `{{message}}`) inserts user-supplied text into terminal commands.
3. **Accidental Destructive Execution**:
   - Tapping a snippet or history command on a mobile screen could immediately run a destructive command (`git push --force`, `rm -rf`) if newline (`\n`) is appended unconditionally or if placeholder values aren't filled.
4. **Denial of Service (DoS) on Fuzzy Search**:
   - Large shell history files or pathological search queries could block the UI thread or exhaust memory.

---

## 2. Security Controls Verified

### 2.1 Automatic Credential Scrubbing (`crates/core/src/snippets.rs`)
- **Control**: `SECRET_HISTORY_REGEX` (`(?i)((api[_-]?key|secret|token|password|passwd|private[_-]?key)\s*[=:]|authorization:\s*bearer\s+)`) filters every line parsed by `extract_history_commands` before deduplication or return across FFI.
- **Verification**: Unit test `test_extract_history_commands_zsh_and_secret_scrubbing` confirms lines containing `OPENAI_API_KEY=sk-123456789` and `Authorization: Bearer secret_token` are dropped completely.

### 2.2 Parameterized Remote History Fetching (`crates/core/src/session.rs`)
- **Control**: `fetch_remote_shell_history(limit: u32)` clamps `limit` to `[10, 1000]` (`limit.clamp(10, 1000)`) and executes a strictly static shell pipeline reading `$HOME/.zsh_history` or `$HOME/.bash_history` via `tail -n <u32>`. No user-controlled string is interpolated into the remote command.
- **Verification**: Inspected `crates/core/src/session.rs` lines 470–495; `safe_limit` is a primitive `u32`.

### 2.3 Safe Execution & Placeholder Resolution (`SnippetsSheetView.swift`)
- **Control**:
  - Snippets containing `{{placeholder}}` tokens intercepted via `extractSnippetPlaceholders(command:)` open an interactive parameter sheet before insertion/execution.
  - Users explicitly choose between **Insert Only** (sends command text without `\n` so the user can inspect/edit it in the prompt) and **Run Immediately** (appends `\n`).
- **Verification**: Inspected `SnippetsSheetView.swift`.

### 2.4 Bounded Memory & Non-Blocking Execution
- **Control**:
  - `extract_history_commands` caps output to `max_items` and skips commands > 500 characters.
  - `fetchRemoteHistory()` runs inside a detached `Task` on `TerminalSessionViewModel` without blocking `@MainActor` rendering.

---

## 3. Findings Summary

| ID | Severity | Location | Description | Status |
|----|----------|----------|-------------|--------|
| — | None | — | No CRITICAL, HIGH, or MEDIUM vulnerabilities identified. | CLEAR |

**Verdict**: **CLEAR** — Ready for merge.
