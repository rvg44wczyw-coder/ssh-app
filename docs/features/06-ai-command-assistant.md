# Feature 06: AI Command Prompt & Shell Assistant (Milestone M9)

**Document Slug:** `06-ai-command-assistant`  
**Target Milestone:** Milestone M9  
**Participating Roles:** `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`  
**Hacker Gate Required:** Yes (Mandatory: touches shell command generation, remote host context reading, LLM prompting, and API key management)  

---

## 1. Problem Statement & Scope

### 1.1 The Problem
When operating a remote MacBook or Linux development workstation from a mobile device (iPhone / Android) over SSH, typing complex terminal commands is slow, error-prone, and frustrating:
- On mobile virtual keyboards, typing flags, quotes, pipes, nested bash expansions (`$(...)`), and long file paths is awkward.
- Developers frequently remember *what* they want to achieve (e.g. *"find docker containers consuming >1GB RAM"*, *"revert last commit but keep changes staged"*, *"kill whatever is listening on port 5173"*) but not the exact flags or command syntax for the target operating system.
- Executing AI-generated commands without inspection is dangerous; hallucinated flags or destructive commands (`rm -rf`, `git reset --hard`) could destroy remote codebases.

### 1.2 Scope & Boundaries
- **In Scope**:
  1. **Rust Core AI Assistant Engine**:
     - **Context Assembly**: Extracting environment context (target OS `macOS`/`Linux`, `$SHELL` `zsh`/`bash`/`fish`, recent terminal output tail) and assembling structured system/user prompts.
     - **Safety & Risk Classification Engine**: Deterministic AST/regex analyzer classifying commands into `Safe`, `Elevated`, `Caution`, or `Destructive`, generating clear human warnings.
     - **LLM Query Client**:
       - *Local Host Provider*: Queries Ollama (`http://127.0.0.1:11434`) running on the remote host directly through SSH exec or port-forwarded loopback channel.
       - *Cloud Provider*: Queries OpenAI / Anthropic / compatible REST endpoints using standard JSON chat completions format.
     - **Response Parser**: Extracts raw executable commands from markdown blocks, parses explanations, and strips formatting artifacts.
  2. **UniFFI Bridge**:
     - Exported records, enums, and `AiAssistant` methods exposed to Swift and Kotlin.
  3. **iOS (SwiftUI) UI**:
     - Quick accessory keyboard button `[ 🪄 ]` in the accessory row.
     - Interactive `AiAssistantSheetView`:
       - Text input + Speech-to-text dictation button (`SFSpeechRecognizer`).
       - Mode picker (Local Host Ollama vs Cloud API).
       - Context preview toggle (shows what terminal lines are sent to LLM).
       - Command suggestion card with syntax highlighting, risk badge, and explanation.
       - Action buttons: `[ Insert Command ]`, `[ Run Immediately ]`, `[ Cancel ]`.
  4. **Android (Jetpack Compose) UI**:
     - Accessory keyboard `[ 🪄 ]` button.
     - Compose `AiAssistantDialog`:
       - Speech dictation via `RecognizerIntent` / Android SpeechRecognizer.
       - Syntax-highlighted suggestion card with risk badges.
       - Insertion and direct execution actions.
  5. **Zero Battery Drain**:
     - AI requests are ephemeral. No persistent network background tasks or background socket listeners remain open when the app is backgrounded.

- **Out of Scope**:
  - Full multi-turn chat UI (reserved for Milestone M10: Structured Transcript & Dual-Mode UI).
  - Fine-tuning or bundling local on-device LLM models inside the iOS/Android app bundle.

---

## 2. Domain Models & Protocol Contracts

### 2.1 Rust Core Domain Models (`crates/core/src/ai.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Risk level classification for an AI-generated shell command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AiRiskLevel {
    /// Read-only or benign operations (ls, grep, cat, docker ps, git status).
    Safe,
    /// Requires elevated root/admin privileges (sudo, doas, chown, chmod).
    Elevated,
    /// Modifies files, services, or repository state (git checkout, kill, systemctl stop).
    Caution,
    /// High-risk or irreversible destructive operations (rm -rf, dd, mkfs, git reset --hard, dropdb).
    Destructive,
}

/// LLM provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiProviderConfig {
    /// "ollama" (remote host), "openai_compatible", or "anthropic".
    pub provider_type: String,
    /// Endpoint URL (e.g. "http://127.0.0.1:11434" or "https://api.openai.com/v1").
    pub endpoint_url: String,
    /// Model name (e.g. "qwen2.5-coder:7b", "llama3.2", "gpt-4o-mini").
    pub model_name: String,
    /// Optional API key for cloud providers (empty for local Ollama).
    pub api_key: Option<String>,
}

/// Request parameters for generating a shell command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiCommandRequest {
    /// Natural language prompt from the user (e.g. "find all files modified today").
    pub user_prompt: String,
    /// Operating system of the remote host ("macOS", "Linux", "Unknown").
    pub target_os: String,
    /// Remote login shell ("zsh", "bash", "fish", "sh").
    pub shell_name: String,
    /// Current working directory if known (e.g. "~/projects/backend").
    pub cwd: Option<String>,
    /// Recent terminal output lines to provide context (max 30 lines).
    pub terminal_context: Vec<String>,
}

/// Verified command suggestion returned to mobile UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiCommandSuggestion {
    /// Clean, ready-to-run shell command line string.
    pub command: String,
    /// Brief explanation of how the command works and what flags mean.
    pub explanation: String,
    /// Risk assessment level.
    pub risk_level: AiRiskLevel,
    /// Specific risk warnings if applicable (e.g. "Contains recursive deletion flag -r").
    pub warnings: Vec<String>,
}
```

### 2.2 UniFFI Interface

```rust
#[uniffi::export]
pub fn assemble_ai_system_prompt(target_os: &str, shell_name: &str, cwd: Option<String>) -> String;

#[uniffi::export]
pub fn parse_ai_response(raw_llm_response: &str) -> Result<AiCommandSuggestion, SshCoreError>;

#[uniffi::export]
pub fn classify_command_risk(command: &str) -> (AiRiskLevel, Vec<String>);

#[uniffi::export]
pub fn sanitize_shell_command(command: &str) -> String;
```

Additionally, on `SshSessionHandle`:
```rust
#[uniffi::export]
impl SshSessionHandle {
    /// Queries Ollama running locally on the remote host over the existing authenticated SSH session.
    pub async fn query_host_ollama(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<String, SshCoreError>;
}
```

---

## 3. Safety & Command Risk Classification Engine

To protect users against accidental system corruption or AI hallucinations, the Rust Core classifies every generated command through deterministic safety rules before presentation to the user:

### 3.1 Destructive Risk Patterns (`AiRiskLevel::Destructive`)
- `rm\s+-[a-zA-Z]*r[a-zA-Z]*f?` or `rm\s+-[a-zA-Z]*f[a-zA-Z]*r?` (`rm -rf`, `rm -r -f`)
- `mkfs`, `fdisk`, `parted`, `dd\s+if=`
- `>\s*/dev/sd[a-z]` or `>\s*/dev/nvme`
- `git\s+reset\s+--hard`
- `git\s+clean\s+-[a-zA-Z]*f`
- `drop\s+database`, `drop\s+table`, `truncate\s+table` (SQL)
- `kill\s+-9\s+-1` or `killall\s+-9`

### 3.2 Elevated Risk Patterns (`AiRiskLevel::Elevated`)
- `sudo\s+`, `doas\s+`, `su\s+`
- `chown\s+-[a-zA-Z]*R`, `chmod\s+777`, `chmod\s+-[a-zA-Z]*R`
- `systemctl\s+(stop|disable|restart|mask)`
- `iptables`, `ufw\s+disable`, `pfctl`

### 3.3 Caution Patterns (`AiRiskLevel::Caution`)
- `kill\s+`, `pkill\s+`
- `docker\s+(rm|rmi|stop|system\s+prune)`
- `git\s+checkout\s+\.`, `git\s+restore\s+\.`
- `npm\s+uninstall`, `pip\s+uninstall`
- Writing/truncating redirects (`>\s*[^&]`)

### 3.4 Safe Patterns (`AiRiskLevel::Safe`)
- Pure read queries: `ls`, `grep`, `find`, `cat`, `head`, `tail`, `less`, `awk`, `sed` (without `-i`), `stat`, `file`.
- Repository status: `git status`, `git log`, `git diff`, `git branch`.
- System inspection: `ps`, `top`, `htop`, `df`, `du`, `uname`, `which`, `env`, `uptime`, `whoami`.
- Network inspection: `netstat`, `lsof -i`, `ss`, `ping -c`, `curl -I`.

---

## 4. Prompt Engineering & Response Parsing

### 4.1 System Prompt Architecture
```text
You are an expert UNIX shell command assistant for {TARGET_OS} running {SHELL_NAME}.
The user will describe a task. Generate EXACTLY ONE shell command to achieve the goal.

Constraints:
1. Output format MUST strictly follow:
COMMAND: <single executable command or pipeline>
EXPLANATION: <concise 1-2 sentence explanation of flags and operation>
2. Do NOT wrap the command in markdown code fences.
3. Tailor the command to {TARGET_OS} (e.g. use BSD tools for macOS and GNU tools for Linux).
4. Never include interactive confirmation prompts if non-interactive flags exist, but avoid unnecessary destructive flags.
```

### 4.2 Robust Parser Logic
`parse_ai_response` parses both structured `COMMAND:` / `EXPLANATION:` output and markdown code fences (`` `command` `` or ````bash ... ````) if the LLM deviates from instructions, guaranteeing that the extracted command is free of markdown artifacts and backticks.

---

## 5. UI Architecture & Mobile Integration

### 5.1 Accessory Keyboard `[ 🪄 ]` Button
- Located on `KeyboardAccessoryRow` on both iOS and Android.
- Tapping `[ 🪄 ]` opens the modal AI Assistant sheet without closing or disrupting the active terminal session.

### 5.2 Speech-to-Text Dictation
- **iOS**: Uses `SFSpeechRecognizer` with permission check (`NSSpeechRecognitionUsageDescription`, `NSMicrophoneUsageDescription`). Users can speak their intent (e.g. *"покажи какие порты сейчас заняты"*) which is transcribed live into the prompt field.
- **Android**: Uses `RecognizerIntent.ACTION_RECOGNIZE_SPEECH` / `SpeechRecognizer`.

### 5.3 Confirmation & Actions
- The user is presented with:
  1. **Risk Badge**:
     - Green: `[ 🟢 Safe ]`
     - Yellow: `[ 🟡 Caution: Modifies files or state ]`
     - Orange: `[ 🟠 Elevated: Requires root/sudo ]`
     - Red: `[ 🔴 Destructive: Irreversible operation ]`
  2. **Code Box**: Syntax-highlighted monospaced command box with a quick `[ Copy ]` button.
  3. **Explanation Box**: Brief explanation of arguments and flags.
  4. **Action Buttons**:
     - `[ Insert into Terminal ]`: Sends text without `\r`, letting the user inspect or edit in the prompt before executing.
     - `[ Run Command ⏎ ]`: Sends command bytes + `\r` directly to the tmux session.
     - `[ Cancel / Dismiss ]`: Closes dialog without sending input.

---

## 6. Security Considerations & Hacker Gate

1. **Prompt Injection Resistance**:
   - The user prompt is strictly isolated in the user prompt section and cannot override the safety classifier.
   - The safety risk classifier is **pure Rust deterministic regex/AST analysis**, completely independent of the LLM's own assessment. Even if an LLM claims a command is "safe", our engine flags `rm -rf` as `Destructive`.
2. **Zero Unconfirmed Remote Execution**:
   - Generated commands are **never** executed automatically. The user must explicitly press `[ Run Command ]` or `[ Insert ]`.
3. **API Key Security**:
   - If Cloud LLM API keys are configured, they are saved exclusively in:
     - iOS: Keychain with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
     - Android: `EncryptedSharedPreferences` backed by Android KeyStore.
   - API keys are never logged in terminal output, system logs, or error messages.
4. **Local Host Privacy**:
   - When using Host Ollama, queries stay strictly on the local machine or over encrypted Tailscale SSH. Zero tokens are sent to external third-party servers.

---

## 7. Verification & Testing Criteria

1. **Rust Core Unit Tests (`crates/core/src/ai.rs`)**:
   - `test_assemble_ai_system_prompt`: Verifies OS and shell injection in prompt text.
   - `test_parse_structured_ai_response`: Verifies parsing of `COMMAND:` / `EXPLANATION:`.
   - `test_parse_markdown_ai_response`: Verifies extracting command from markdown fences.
   - `test_risk_classification_safe`: Verifies `ls -la`, `git status`, `grep foo file`.
   - `test_risk_classification_destructive`: Verifies `rm -rf /`, `git reset --hard`, `mkfs.ext4`.
   - `test_risk_classification_elevated`: Verifies `sudo systemctl restart`, `chmod 777`.
   - `test_risk_classification_caution`: Verifies `pkill -f node`, `docker rm -f`.
   - `test_sanitize_shell_command`: Verifies stripping trailing newlines and escape sequences.
2. **Swift Package Tests**:
   - Binding roundtrip and safety classification verification via Swift.
3. **UI Build Verification**:
   - iOS: `xcodebuild -destination 'generic/platform=iOS'` compiles with zero errors.
   - Android: `./gradlew assembleDebug` compiles with zero errors.
4. **Coverage Target**:
   - >= 80% line coverage in `crates/core/src/ai.rs`.
