# Feature 10: Remote File Browser & Quick Code Viewer

- **Status**: Implemented & Verified
- **Milestone**: M13
- **Layers**: `crates/core`, `packages/SshCoreBridge`, `ios/`

---

## 1. Objective & User Experience

When managing AI coding agents (Claude Code, Google Antigravity, local LLMs) from an iPhone, developers frequently need to inspect files created or modified by an agent, check directory structures, copy a file path into the terminal prompt, or `cd` the active terminal into a specific project folder without typing long paths on a mobile keyboard.

### Key Capabilities
1. **Tmux-Aware Starting Directory**:
   - Automatically detects the active `tmux` pane's current working directory (`tmux display-message -p -t <session> '#{pane_current_path}'`), falling back to `$HOME`.
2. **Fast, Safe Directory Listing**:
   - Lists directories first, then files alphabetically, including entry type (`Directory`, `File`, `Symlink`), byte size, and hidden file toggle.
3. **Safe Code & Text Preview**:
   - Previews text/source files up to `128 KB` (or `500` lines) with line numbers, syntax-friendly monospaced rendering, binary file detection, and one-tap Copy Content / Insert Path.
4. **Terminal Integration Actions**:
   - **`cd` Here**: Sends `cd '<escaped_path>'\n` directly to the active terminal session.
   - **Insert Path**: Inserts the safely shell-quoted path into the active terminal input without executing.

---

## 2. Security & Path Validation (`validate_remote_file_path`)

- **NUL & Control Character Rejection**: Any path containing `\0`, `\r`, or `\n` is rejected before shell quoting.
- **Sensitive Credential Path Blocklist**:
  - Rejects paths targeting `.ssh` private keys, `.aws`, `.gnupg`, `.kube/config`, `/etc/shadow`, `/etc/master.passwd`, or `.env` secret files when reading content (`validate_remote_file_read_path`), while allowing safe directory navigation.
- **Strict POSIX Single-Quote Escaping**:
  - All remote shell commands use `shell_escape_single_quotes` (`'` -> `'\''`) so arbitrary filenames with spaces, `$()`, backticks, or semicolons can never trigger command injection.

---

## 3. Rust Core Architecture (`crates/core/src/file_browser.rs`)

### 3.1 UniFFI Domain Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFileKind {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub kind: RemoteFileKind,
    pub size_bytes: u64,
    pub is_hidden: bool,
    pub permissions: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteDirectoryListing {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub entries: Vec<RemoteFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteFilePreview {
    pub path: String,
    pub filename: String,
    pub content: String,
    pub size_bytes: u64,
    pub is_truncated: bool,
    pub is_binary: bool,
}
```

### 3.2 Pure Helper & Parser Functions
- `validate_remote_dir_path(path: &str) -> Result<String>`
- `validate_remote_file_read_path(path: &str) -> Result<String>`
- `parse_remote_directory_listing(raw_output: String, include_hidden: bool) -> RemoteDirectoryListing` (UniFFI-exported; drops entries/cwd containing control characters)
- `build_file_preview_from_bytes(path: &str, raw_bytes: &[u8], max_bytes: usize) -> RemoteFilePreview`
- `build_list_directory_command`, `build_resolve_path_command`, `build_read_file_command` (regular files only), `quote_remote_path_for_shell` (UniFFI-exported)

### 3.3 `SshSessionHandle` Methods
- `pub async fn list_remote_directory(&self, path: Option<String>, include_hidden: bool) -> Result<RemoteDirectoryListing, SshError>`
  - If `path` is `None` or empty, resolves the active tmux session's `#{pane_current_path}` (or `$HOME`).
  - Runs a single-shot POSIX command that prints `realpath`/`pwd -P` on line 1 followed by `ls -la` in a deterministic portable format (`LC_ALL=C ls -la`).
- `pub async fn read_remote_file_preview(&self, path: String, max_bytes: u32) -> Result<RemoteFilePreview, SshError>`
  - Validates `path` via `validate_remote_file_read_path`.
  - Reads up to `max_bytes` (capped at `131_072` bytes) using `head -c` over an ephemeral SSH `exec` channel.

---

## 4. iOS UI (`FileBrowserSheetView.swift`)

- Triggered via a folder toolbar button (`folder.fill`) in `MainTerminalView`.
- Uses the unified `ActiveModalSheet.fileBrowser` enum case and `onClose` callback.
- Features:
  - Path header with `..` (Up), `~` (Home), `cd Here` (sends `cd` to terminal), and Hidden Files toggle (`eye` / `eye.slash`).
  - Search filter bar to filter entries in the current directory.
  - Tap a directory row to drill down; tap a file row to open the Code/Text Preview sheet with line numbers, Copy Content, and Insert Path buttons.

---

## 5. Hardening (from `rev` + `hacker`, see `docs/security-findings/2026-10-16-remote-file-browser-audit.md`)
- Symlinks are resolved remotely (`realpath`/`pwd -P`) and the **resolved** path is re-validated against the blocklist, for both listing and file reads.
- Reads use `[ -f ]` + `head -c` so FIFOs/devices cannot hang the channel.
- Blocklist: `.env*`, `.netrc`, `.npmrc`, `.pgpass`, `.pypirc`, `.git-credentials`, `*.pem`, `*.key`, SSH/AWS/GPG/kube dirs.
- Names with control characters are never listed (prevents keystroke injection via Insert Path / `cd Here`).
