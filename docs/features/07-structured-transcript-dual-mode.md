# Feature 07: Structured Transcript & Dual-Mode UI (Milestone M10)

**Document Slug:** `07-structured-transcript-dual-mode`  
**Target Milestone:** Milestone M10  
**Participating Roles:** `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`  
**Hacker Gate Required:** Yes (Mandatory: touches remote transcript file parsing, untrusted JSONL ingestion, ANSI stripping, and remote path validation)  

---

## 1. Problem Statement & Scope

### 1.1 The Problem
When running autonomous AI coding agents (Claude Code, Google Antigravity, Ollama-based agent frameworks, Aider) on a remote MacBook or Linux host over SSH, mobile terminal ergonomics suffer:
- Terminal viewports on mobile phones (iPhone / Android) are small (~40-80 columns wide).
- AI agents emit massive bursts of ANSI progress bars, spinning loaders, intermediate diffs, and verbose tool calls that cause text wrapping and push the user's initial prompts out of the scrollback buffer.
- Reading and verifying multi-step reasoning, tool invocations (`run_command: cargo test`, `read_file`, `write_file`), and tool execution results inside a raw terminal requires tedious touch-scrolling.
- However, developers still need the raw terminal for direct shell commands, interactive input (`y/n`, `sudo` password), or tmux pane management.

### 1.2 The Solution: Dual-Mode UI (Raw Terminal ↔ Structured Chat View)
Provide a seamless, real-time dual-mode switch inside the mobile session:
1. **Raw Terminal Mode (`📟 Terminal`)**:
   - Unaltered `SwiftTerm` (iOS) and `TerminalView` (Android) PTY connection attached to the remote tmux session.
   - Low-latency keystrokes, accessory keyboard bar, TrueColor rendering.
2. **Structured Chat Mode (`💬 Chat`)**:
   - High-density native mobile card UI displaying the agent's work stream.
   - Clean message cards for User Prompts and Assistant Responses with syntax-highlighted code blocks.
   - Collapsible **"Thinking"** accordions showing inner reasoning without cluttering the screen.
   - **Tool Call Cards** detailing tool names (`run_command`, `write_file`, `read_file`), target parameters, status badges (`Running`, `Done`, `Error`), and expandable output sheets.
   - **Interactive Approvals**: Actionable banners for pending agent confirmations (`[ Approve ]` / `[ Reject ]`).

### 1.3 Scope & Boundaries
- **In Scope**:
  1. **Rust Core Transcript Engine (`crates/core/src/transcript.rs`)**:
     - Streaming & batch JSONL parser supporting both **Google Antigravity** (`step_index`, `source`, `type`, `thinking`, `tool_calls`) and **Claude Code** / OpenAI-compatible tool call formats.
     - Universal normalized domain models (`TranscriptTurn`, `TranscriptToolCall`, `TranscriptTurnType`, `TranscriptStatus`).
     - Safe ANSI escape sequence stripping from text content.
     - Memory-bounded ingestion (max file size and chunk caps to prevent OOM DOS).
     - Remote transcript reader via ephemeral SSH channel with strict path traversal validation.
     - Automatic remote transcript locator (`detect_active_transcript_path`).
  2. **UniFFI Bridge**:
     - Exported records, enums, parser functions, and session methods for Swift and Kotlin.
  3. **iOS (SwiftUI) UI**:
     - Mode toggle in the top bar: `[ 💬 Chat / 📟 Term ]`.
     - `StructuredTranscriptView.swift` with auto-scrolling `ScrollViewReader`, message bubbles, collapsible thinking accordions, tool cards, and pull-to-refresh.
  4. **Android (Jetpack Compose) UI**:
     - Mode toggle in the top app bar: `[ 💬 Chat | 📟 Term ]`.
     - `StructuredTranscriptScreen.kt` with `LazyColumn`, animated expansion cards, and tool badges.
  5. **Zero Battery Drain**:
     - Transcript reading is on-demand or throttled; all SSH channels and timers are immediately torn down on app backgrounding (`sceneDidEnterBackground`).

- **Out of Scope**:
  - Offline local agent fine-tuning.
  - Multi-agent collaboration graph editing.

---

## 2. Domain Models & Protocol Contracts

### 2.1 Rust Core Domain Models (`crates/core/src/transcript.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Status of an agent turn or tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum TranscriptStatus {
    Running,
    Done,
    Error,
}

/// Type of turn in the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum TranscriptTurnType {
    User,
    Assistant,
    System,
    ApprovalRequest,
}

/// Represents an individual tool call executed by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TranscriptToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments_summary: String,
    pub full_arguments_json: Option<String>,
    pub result: Option<String>,
    pub is_error: bool,
    pub status: TranscriptStatus,
}

/// Represents a single conversation turn in the structured transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TranscriptTurn {
    pub id: String,
    pub step_index: u64,
    pub turn_type: TranscriptTurnType,
    pub timestamp: Option<String>,
    pub content: Option<String>,
    pub thinking: Option<String>,
    pub tool_calls: Vec<TranscriptToolCall>,
    pub status: TranscriptStatus,
}
```

### 2.2 UniFFI Interface Signatures

```rust
#[uniffi::export]
pub fn parse_transcript_jsonl(raw_jsonl: String) -> Result<Vec<TranscriptTurn>, TranscriptError>;

#[uniffi::export]
pub fn strip_ansi_codes(input: String) -> String;

impl SessionManager {
    #[uniffi::method]
    pub async fn read_remote_transcript(
        &self,
        remote_path: String,
    ) -> Result<Vec<TranscriptTurn>, SshCoreError>;

    #[uniffi::method]
    pub async fn detect_active_transcript_path(
        &self,
        base_dir: Option<String>,
    ) -> Result<Option<String>, SshCoreError>;
}
```

### 2.3 Error Definitions (`crates/core/src/error.rs`)

```rust
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TranscriptError {
    #[error("Failed to parse JSONL line: {0}")]
    JsonParseError(String),
    #[error("File size exceeds safety limit ({0} bytes)")]
    PayloadTooLarge(usize),
    #[error("Path validation failed: {0}")]
    InvalidPath(String),
    #[error("Transcript IO error: {0}")]
    IoError(String),
}
```

---

## 3. Remote Environment & Transcript Protocols

### 3.1 Transcript Discovery Protocol
AI agents save session transcripts to standardized locations on the remote host:
1. **Google Antigravity (`agy`)**:
   - `~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/transcript.jsonl`
   - Active transcript symlink or latest directory sorted by mtime:
     `ls -td ~/.gemini/antigravity-cli/brain/*/ 2>/dev/null | head -n 1`
2. **Claude Code**:
   - `~/.claude/projects/*/transcript.jsonl` or `.claude/transcript.jsonl` in workspace root.
3. **Generic Project Transcripts**:
   - `<workspace>/.agents/transcript.jsonl` or `transcript.jsonl`.

The remote detection query executed over SSH:
```bash
zsh -l -c '
if [ -d "$HOME/.gemini/antigravity-cli/brain" ]; then
  LATEST=$(ls -td "$HOME/.gemini/antigravity-cli/brain"/* 2>/dev/null | head -n 1)
  if [ -n "$LATEST" ] && [ -f "$LATEST/.system_generated/logs/transcript.jsonl" ]; then
    echo "$LATEST/.system_generated/logs/transcript.jsonl"
    exit 0
  fi
fi
if [ -d "$HOME/.claude/projects" ]; then
  LATEST_CLAUDE=$(ls -td "$HOME/.claude/projects"/*/*.jsonl 2>/dev/null | head -n 1)
  if [ -n "$LATEST_CLAUDE" ]; then
    echo "$LATEST_CLAUDE"
    exit 0
  fi
fi
'
```

### 3.2 Safe File Ingestion
- Ephemeral SSH command reads the tail (last 500 lines or last 1MB):
  `tail -n 500 <remote_path>`
- Prevents reading huge multi-gigabyte log dumps into mobile RAM.
- Strict path validation ensures `remote_path` is restricted to user home subdirectories or project folders, preventing arbitrary file reads (e.g., `/etc/shadow`, `~/.ssh/id_ed25519`).

---

## 4. State Machines & Lifecycle

```
[ App Launch / Connected ]
         │
         ├── Default: [ Raw Terminal Mode ] (Active PTY Stream)
         │        │
         │   Toggle button pressed
         │        │
         ▼        ▼
   [ Structured Chat Mode ]
         │
         ├── Fetch / Tail remote transcript via SSH exec channel
         ├── Parse JSONL into Vec<TranscriptTurn>
         ├── Render SwiftUI / Compose Card Canvas
         │
         ├── Switch back to [ Raw Terminal Mode ] (PTY buffer still intact)
         │
         └── On `sceneDidEnterBackground` (iOS) / `onStop` (Android):
                 - Socket & SSH channels closed immediately (Zero Battery Drain)
```

---

## 5. Security & Hacker Considerations

1. **Path Traversal & Arbitrary File Read**:
   - Remote path must be normalized and checked against a blacklist (`/etc/`, `/var/`, `/dev/`, `~/.ssh/`).
   - Semicolons and shell expansions in paths are disallowed or shell-quoted.
2. **Untrusted JSONL Payload Safety**:
   - Untrusted LLM output may contain malicious ANSI control sequences (e.g. cursor moves, terminal renames, OSC 52 clipboard injection). The parser strips all ANSI escape sequences before returning strings to Swift/Kotlin.
3. **Memory Limits (Denial of Service)**:
   - A maximum payload ceiling (5MB) is enforced in `crates/core` to prevent Out-Of-Memory termination on low-memory mobile devices.

---

## 6. Verification & Testing Criteria

1. **Rust Core Tests (`crates/core/src/transcript.rs`)**:
   - Test Antigravity format parsing (user input, planner response, thinking, tool calls).
   - Test Claude Code format parsing.
   - Test ANSI code stripping (`strip_ansi_codes`).
   - Test malformed JSON line skipping without panicking.
   - Test max payload limits.
2. **Swift & Android Build Verification**:
   - `build-xcframework.sh` and `build-android.sh` regenerate bindings cleanly.
   - `xcodebuild` succeeds on iOS target.
   - `./gradlew assembleDebug` succeeds on Android target.
3. **Target Coverage**: >= 85% for `transcript.rs`.
