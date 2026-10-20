# Feature 40: Voice Input & Technical Agent Prompt Dictation

- **Slug**: `agent-voice-dictation`
- **Target Release**: Milestone 29 (Mobile Ergonomics & Natural Language Steering)
- **Roles**: `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (audio privacy, local on-device speech processing, microphone entitlement, prompt injection prevention)
- **Status**: Reviewed: fix the normalizer and the fallback (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Formulating detailed instructions for AI agents on a virtual mobile keyboard is tedious and error-prone. Engineers often want to speak their intent while away from their desk:
- *"Add a unit test for the token refresh logic in auth.rs and handle expired tokens with a custom error."*
However, standard iOS voice dictation struggles with programming syntax:
- It produces spaces inside file names (`auth . rs` instead of `auth.rs`).
- It spells code tokens in plain words (`camel case`, `equals equals`, `dash dash flag`).
- It inserts unwanted capitalizations and punctuation quirks.

### 1.2 In Scope
- Dedicated microphone button `[ mic.fill ]` inside the Agent Feed composer bar.
- 100% on-device speech recognition via Apple's `SFSpeechRecognizer` with `requiresOnDeviceRecognition = true` (zero audio leaves the device; works completely offline).
- Technical syntax post-processor (Rust Core or Swift utility):
  - File extension detection (`dot r s` $\rightarrow$ `.rs`, `dot t s` $\rightarrow$ `.ts`, `dot p y` $\rightarrow$ `.py`).
  - Flag normalization (`dash dash force` $\rightarrow$ `--force`, `minus p` $\rightarrow$ `-p`).
  - Code casing converter (`camel case getUserProfile` $\rightarrow$ `getUserProfile`, `snake case auth_token` $\rightarrow$ `auth_token`).
  - Common command token replacements (`git checkout`, `cargo build`, `npm run`, `kubectl`).
- Real-time partial transcript stream displayed as an interactive editable overlay above the keyboard.
- Quick prompt action chips generated from dictated text (`Review`, `Fix`, `Explain`, `Refactor`).

### 1.3 Out of Scope
- Sending raw audio files to remote servers or third-party cloud APIs (Whisper API / Google Cloud). Audio stays strictly on the iOS device.

---

## 2. Technical Post-Processing Engine

### 2.1 Rule-Based Technical Lexicon Normalizer (`crates/core/src/dictation.rs`)

```rust
use regex::Regex;
use std::sync::LazyLock;

static RE_FILE_EXTENSIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:dot\s+|point\s+)(rs|ts|tsx|js|jsx|py|go|swift|c|cpp|h|json|toml|yaml|yml|md|sh)\b")
        .expect("valid regex")
});

static RE_CLI_FLAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:dash\s+dash|minus\s+minus)\s+([a-z0-9\-_]+)\b")
        .expect("valid regex")
});

static RE_SINGLE_FLAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:dash|minus)\s+([a-z0-9])\b")
        .expect("valid regex")
});

pub fn normalize_technical_dictation(raw_text: &str) -> String {
    let mut normalized = raw_text.to_string();

    // 1. Normalize file extensions: "auth dot rs" -> "auth.rs"
    normalized = RE_FILE_EXTENSIONS.replace_all(&normalized, ".$1").to_string();

    // 2. Normalize flags: "dash dash force" -> "--force"
    normalized = RE_CLI_FLAGS.replace_all(&normalized, "--$1").to_string();

    // 3. Normalize single flags: "minus v" -> "-v"
    normalized = RE_SINGLE_FLAGS.replace_all(&normalized, "-$1").to_string();

    // 4. Clean spacing around dots in paths
    normalized = normalized.replace(" / ", "/").replace(" . ", ".");

    normalized
}
```

### 2.2 UniFFI Interface

```rust
#[uniffi::export]
pub fn sanitize_agent_voice_prompt(raw_transcript: String) -> String {
    normalize_technical_dictation(&raw_transcript)
}
```

---

## 3. UI/UX Interaction Design (No Emojis)

```text
┌────────────────────────────────────────────────────────┐
│ [● Recording... 00:04]                    [waveform]   │
│ "add a unit test for token refresh in auth.rs"         │
├────────────────────────────────────────────────────────┤
│ Quick actions: [ Fix ] [ Test ] [ Explain ] [ Refactor]│
├────────────────────────────────────────────────────────┤
│ [ ✕ Cancel ]                           [ ✓ Insert ]    │
└────────────────────────────────────────────────────────┘
```

1. **Tap & Hold or Tap to Toggle**: Tapping `[ mic.fill ]` activates recording with haptic feedback.
2. **Audio Waveform**: A lightweight CoreAnimation / SwiftUI audio level visualizer monitors ambient volume.
3. **Editable Review**: The sanitized text appears in the composer. The developer can edit manually before sending to the agent.

---

## 4. Security & Privacy Considerations (Hacker Gate)

| Aspect | Policy |
| :--- | :--- |
| **Microphone Entitlement** | `NSMicrophoneUsageDescription` states clearly: "Microphone is used strictly for on-device prompt dictation. No audio is recorded or sent over the network." |
| **Speech Privacy** | `SFSpeechAudioBufferRecognitionRequest.requiresOnDeviceRecognition = true`. If on-device recognition is unavailable on older iOS models, dictation falls back to standard keyboard dictation without third-party audio transmission. |
| **Prompt Injection Protection** | Sanitizer strips control characters (`\x00..\x1F`, raw ANSI escapes) before insertion into the composer to prevent escape sequence injection into the active terminal pane. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_normalize_file_extensions`: verifies `main dot py` $\rightarrow$ `main.py`.
   - `test_normalize_flags`: verifies `dash dash all` $\rightarrow$ `--all`.
   - `test_strip_dangerous_ansi_escapes`: removes escape codes from dictated strings.
2. **Swift UI Tests**:
   - Permission grant/denial flows for microphone and speech recognizer.
   - Composer insertion and undo/redo behavior.

---

## Review (`rev`, 2026-10-20)

- [High] `"main dot py"` becomes `"main .py"`: the space before `dot` stays. The regex must consume it (`\s*\b(?:dot|point)\s+(ext)\b` replaced by `.ext`). The test `main dot py -> main.py` would fail as written.
- [High] The fallback to ordinary keyboard dictation sends audio to Apple. That contradicts "no audio leaves the device"; if on-device recognition is unavailable, disable the microphone button.
- [Medium] `camel case` and `snake case` are in scope but have no rule or test.
- [Medium] `.expect("valid regex")` in non-test code; build the regexes in a way that cannot panic at runtime or cover them with a test.
- [Medium] "dash a" also matches ordinary English; only convert after a command-like word or inside the composer's "code" mode.
- [Medium] Russian speech with English terms is the real use case; the lexicon needs Russian forms ("точка эр эс") or the limitation must be stated. On-device recognition also has a per-request time limit.
- [Low] The role list lacks `rust-core-dev`, although the code lives in `crates/core`.
