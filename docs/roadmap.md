# Project Roadmap & Implementation Milestones

## Milestones Overview

| Milestone | Scope | Status | Roles Involved |
| :--- | :--- | :--- | :--- |
| **M0: Project Setup & Skills** | Skill structure, repository initialization, AGENTS.md | Completed | Orchestrator |
| **M1: Rust Core Architecture & Design Doc** | Architecture spec, domain models, protocol contracts, Zero Battery Drain spec | Completed | `doc-dev`, `rev` |
| **M2: Rust Core Engine Implementation** | `crates/core` with `russh`, `ssh-key`, `tmux`, PTY stream, throttling buffer, unit tests | Completed | `rust-core-dev`, `rev` |
| **M3: UniFFI Bridge & Packaging** | UniFFI proc-macro, Swift bindings, multi-target XCFramework build script, SPM package | Completed | `swift-bridge-dev`, `rev` |
| **M4: iOS App & SwiftTerm Integration** | Xcode project, SwiftUI views, SwiftTerm integration, custom accessory keyboard | Completed | `ios-ui-dev`, `rev` |
| **M5: Multi-Session, Lifecycle & Security Audit** | Multi-tab UI, Zero Battery Drain lifecycle, Keychain integration, Hacker audit | Completed | `ios-ui-dev`, `hacker`, `rev` |
| **M6: Agent Control & Secure Approvals** | Host hooks, Ed25519-signed approvals, Zero-Knowledge APNs wake-up, Diff Viewer | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M7: Android Client Architecture** | UniFFI Kotlin bindings, AndroidKeyStore Ed25519, Jetpack Compose, Biometric auth | Completed | `doc-dev`, `rust-core-dev`, `android-ui-dev`, `rev`, `hacker` |
| **M8: In-App Web Preview & SSH Port Forwarding** | Direct TCP/IP port forwarding in Rust Core (`russh`), mobile in-app WebView (`127.0.0.1:3000`), DevTools console drawer | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker` |
| **M9: AI Command Prompt & Shell Assistant** | Natural language shell helper, Ollama/LLM client in core, command sanitization, `[ 🪄 ]` keyboard action | Planned | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker` |
| **M10: Structured Transcript & Dual-Mode UI** | Real-time `transcript.jsonl` parser in Rust, dual-mode UI (Raw Terminal $\leftrightarrow$ Structured Chat View) | Planned | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev` |
| **M11: Smart Snippets & Fuzzy History Search** | Rust Core SQLite/JSON storage, fuzzy search (`nucleo`/`skim`), mobile autocomplete bar | Planned | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev` |

---

## Detailed Feature Log

### Feature 01: Core Architecture & SSH Engine Specification
- **Doc**: `docs/features/01-core-ssh-engine.md`
- **Roles**: `doc-dev`, `rust-core-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-04-audit.md`)
- **Status**: Implemented & Verified with `xcodebuild` (**BUILD SUCCEEDED**)

### Feature 02: Tmux Session Management & Multi-Server Support
- **Doc**: `docs/features/02-tmux-management-and-multi-server.md`
- **Roles**: `doc-dev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-04-tmux-and-multi-server-audit.md`)
- **Status**: Implemented, Built & Installed on iPhone 15 Pro (**BUILD SUCCEEDED**)

### Feature 03: AI Agent Control, Cryptographic Approvals & Zero-Knowledge Wake-up Pings
- **Doc**: `docs/features/03-agent-control-and-secure-approvals.md`
- **Roles**: `doc-dev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-06-agent-control-and-approvals-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented, Verified with `cargo test` (20/20 passed), `swift test` (3/3 passed), and `xcodebuild` (**BUILD SUCCEEDED**)

### Feature 04: Android Client Architecture & Cross-Platform Support
- **Doc**: `docs/features/04-android-support-and-architecture.md`
- **Roles**: `doc-dev`, `rust-core-dev`, `android-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed
- **Status**: Implemented & Verified with Gradle (`./gradlew assembleDebug` **BUILD SUCCESSFUL**)

### Feature 05: In-App Web Preview & SSH Port Forwarding (Localhost Tunnel)
- **Doc**: `docs/features/05-web-preview-and-port-forwarding.md`
- **Scope**:
  - **Rust Core**: Direct TCP/IP port forwarding (`russh::client::Handle::channel_open_direct_tcpip`), local loopback socket proxy on `127.0.0.1`, bidirectional stream pump, tunnel lifecycle management.
  - **Platform (iOS/Android)**: `WKWebView` (iOS) / Jetpack Compose `WebView` (Android) sheet, quick port chips (`3000`, `5173`, `8000`, `8080`), DevTools console drawer with log interceptor, complete background socket teardown.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-08-port-forwarding-and-web-preview-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented & Verified with `cargo test` (21/21 passed), `swift test` (3/3 passed), `xcodebuild` (**BUILD SUCCEEDED**), and `./gradlew assembleDebug` (**BUILD SUCCESSFUL**)

### Feature 06: AI Command Prompt & Shell Assistant
- **Scope**:
  - **Rust Core**: Context assembly (OS, shell, recent terminal history), LLM query (local Ollama over SSH or provider API), shell command sanitization and syntax check.
  - **Platform (iOS/Android)**: Accessory keyboard `[ 🪄 ]` button, speech-to-text dictation via platform APIs (`SFSpeechRecognizer` / Android Speech API), interactive command confirmation dialog.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`
- **Status**: Planned (Milestone M9)

### Feature 07: Structured Transcript & Dual-Mode UI (Chat ↔ Terminal)
- **Scope**:
  - **Rust Core**: Streaming JSONL parser for agent transcripts (Claude Code, Antigravity), structured turn/tool/approval models.
  - **Platform (iOS/Android)**: Dual-mode UI toggle, native SwiftUI/Compose card view for agent dialogue, expandable tool call details.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`
- **Status**: Planned (Milestone M10)

### Feature 08: Smart Snippets & Fuzzy History Search
- **Scope**:
  - **Rust Core**: SQLite/JSON local snippet database, fuzzy matching engine (`nucleo`/`skim`).
  - **Platform (iOS/Android)**: Autocomplete accessory bar, snippet editor sheet, keyboard shortcut triggers.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`
- **Status**: Planned (Milestone M11)




