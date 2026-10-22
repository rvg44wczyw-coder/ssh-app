# Feature 40: Voice Input & Technical Agent Prompt Dictation

- **Slug**: `agent-voice-dictation`
- **Target Release**: Milestone 29 (Mobile Ergonomics & Natural Language Steering)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (audio privacy, local on-device speech processing, microphone entitlement, prompt injection prevention)
- **Status**: Superseded 2026-10-22. Decision: use the iOS keyboard's own dictation in the composer (system component, nothing to build); no custom recorder and no `SFSpeechRecognizer`. The privacy promise "audio never leaves the device" is dropped: on-device or not is the system's choice per language. The technical-word normalizer of section 2 is NOT built: the problems it solves (`auth . rs`, `dash dash force`) were assumed, not measured. Revisit only if dictation on a real device damages file names or flags; the rule table below is the starting point.

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

The normalizer works on whitespace-separated words, not on regular expressions: nothing can fail to compile at run time, so there is no `expect`, and every rule is a plain comparison that a test can pin. Words are compared case-insensitively; the original spelling of everything that is not a rule word is kept.

| Spoken | Written | Rule |
| :--- | :--- | :--- |
| `main dot py` | `main.py` | `dot` or `point` followed by a known extension joins the previous word and the extension with no spaces (the space before `dot` is consumed). Extensions: `rs ts tsx js jsx py go swift c cpp h json toml yaml yml md sh`. |
| `dash dash force` | `--force` | `dash dash` / `minus minus` followed by one flag word (`[a-z0-9_-]+`). |
| `git commit dash m` | `git commit -m` | `dash` / `minus` followed by a single letter or digit **only** when the previous word is a command-like word (`git`, `cargo`, `ls`, `rm`, `cp`, `mv`, `npm`, `make`, `docker`, `tmux`, `ssh`, `grep`, `curl`, any word already starting with `-`) or the composer is in code mode. "dash a" in ordinary prose stays as spoken. |
| `snake case user id` | `user_id` | `snake case` joins the following words (up to the next rule word or the end) with `_`, lower-cased. Without a following word the phrase is left as spoken. |
| `camel case user id` | `userId` | `camel case` joins the following words, first lower-case, the rest capitalised. |
| `slash` between two words | `a/b` | only inside a path-looking run (next to a word containing `.` or `/`), not in prose. |

Russian speech with English terms is the real use case. The lexicon therefore also knows Russian spoken forms: `точка` (as `dot`), `тире тире` (as `dash dash`), `слэш`, `снейк кейс`, `кэмел кейс`, plus extensions spelled by letters (`эр эс` for `rs`, `пай` for `py`). Forms that are not in the table are not guessed: the limitation is stated in the UI help ("works for English terms and the listed Russian forms"). On-device recognition also has a per-request time limit (about one minute): the recorder restarts the request on every limit and joins the pieces.

Control characters (`\x00..\x1F` except space, raw ANSI escapes) are stripped first.

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
| **Speech Privacy** | `SFSpeechAudioBufferRecognitionRequest.requiresOnDeviceRecognition = true`. If on-device recognition is unavailable for the language or the device, the microphone button is disabled with the reason shown; there is no fallback, because the keyboard's own dictation sends audio to Apple and the stated promise is that no audio leaves the device. |
| **Prompt Injection Protection** | Sanitizer strips control characters (`\x00..\x1F`, raw ANSI escapes) before insertion into the composer to prevent escape sequence injection into the active terminal pane. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_normalize_file_extensions`: `main dot py` -> `main.py`, `Main Dot RS` -> `Main.rs`, `dot` without a known extension stays.
   - `test_camel_and_snake_case`: `snake case user id` -> `user_id`, `camel case user id` -> `userId`, a bare `snake case` stays.
   - `test_single_flag_needs_a_command_word`: `git commit dash m` -> `git commit -m`; `dash a thing` in prose stays.
   - `test_russian_forms`: `main точка пай` -> `main.py`.
   - `test_no_panic_on_any_input`: empty, one word, only rule words, very long input (a fuzz-style loop).
   - `test_normalize_flags`: verifies `dash dash all` $\rightarrow$ `--all`.
   - `test_strip_dangerous_ansi_escapes`: removes escape codes from dictated strings.
2. **Swift UI Tests**:
   - Permission grant/denial flows for microphone and speech recognizer.
   - Composer insertion and undo/redo behavior.

---

## Review (`rev`, 2026-10-20) - applied

All items of the first review are folded into sections 2.1, 4 and 5 above (word-based engine instead of regexes, no audio fallback, camel/snake rules with tests, command-word guard for single flags, Russian forms and the time limit). The code lives in `crates/core`, so `rust-core-dev` is part of the chain for this feature: `doc-dev -> rev -> rust-core-dev -> rev -> swift-bridge-dev -> rev -> ios-ui-dev -> rev -> hacker`.
