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
| **M9: AI Command Prompt & Shell Assistant** | Natural language shell helper, Host Ollama client in core, command risk classifier, `[ 🪄 ]` keyboard action | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker` |
| **M10: Structured Transcript & Dual-Mode UI** | Real-time `transcript.jsonl` parser in Rust, dual-mode UI (Raw Terminal $\leftrightarrow$ Structured Chat View) | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M11: Smart Snippets & Fuzzy History Search** | Rust Core snippet catalog & secret-scrubbed history parser, fuzzy search engine, iOS snippets & history sheet | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M12: Remote Host Telemetry & Process Monitor** | CPU/RAM/Disk/Battery/Ollama telemetry collector in Rust Core, process category classifier (`AiAgent`, `DevServer`, `System`), safe process kill (`SIGTERM`/`SIGKILL`), iOS dashboard sheet | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M13: Remote File Browser & Quick Code Viewer** | Tmux-aware directory listing, symlink-safe credential blocklist, 128 KB text preview, `cd` Here / Insert Path, iOS `FileBrowserSheetView` | Completed | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M14: Loopback sshd Integration Tests** | End-to-end tests of exec-based `SshSessionHandle` methods (telemetry, file browser incl. symlink/blocklist checks) against a throwaway `sshd` on `127.0.0.1` | Completed | `rust-core-dev`, `rev` |
| **M15: Agent Hooks & Launch Profiles** | Approval/notification hooks for Claude Code (`PermissionRequest`) and Antigravity (`PreToolUse`), idempotent installer, auto-start launch profiles (work/personal `CLAUDE_CONFIG_DIR`), optional `~/.zshrc` loading + host binary check, hook presets (phone approval / notify only / custom command) with in-app preview of installed vs planned entries, iOS profile & hooks UI | Completed | `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M16: Toolbar Redesign, Server Popup & Host Ollama Check** | One "More" button opening a labeled action panel with long-press hints (replaces 9 toolbar icons and the system overflow), server picker popup (startup, new tab, tmux sheet), host Ollama check (installed / running / models) driving the AI assistant, shared host-probe watchdog | Completed (iOS) | `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M17: Host Approval Queue & Phone Approvals** | Hook-as-broker file queue under `~/.ssh-app/run` (no daemon, no listening socket), SSHSIG-signed Approve/Deny verified on the host with `ssh-keygen -Y verify`, `ctl list/decide/ack/status`, foreground approval card with host-clock countdown in the app, install of the trusted key, approval status in the profiles sheet | Completed (Rust + iOS; live phone test pending) | `doc-dev`, `rev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `rev`, `ios-ui-dev`, `rev`, `hacker` |

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
- **Hacker Gate**: Marked completed, but no findings doc is on record in `docs/security-findings/` (TODO: add or re-run `hacker`)
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
- **Doc**: `docs/features/06-ai-command-assistant.md`
- **Scope**:
  - **Rust Core**: Context assembly (OS detection, `$SHELL`, recent terminal history tail), Host Ollama query (`http://127.0.0.1:11434/api/generate` over SSH), deterministic AST/regex safety classifier (`AiRiskLevel`), markdown code extraction and shell command sanitization.
  - **Platform (iOS/Android)**: Accessory keyboard `[ 🪄 ]` button, top bar assistant action, `AiAssistantSheetView` (iOS SwiftUI) / `AiAssistantDialog` (Android Compose) with risk level badges (`Safe`, `Caution`, `Elevated`, `Destructive`), quick suggestion chips, code box with copy, insert, and run actions.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-08-ai-command-assistant-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented & Verified with `cargo test` (31/31 passed), `swift test` (3/3 passed), `xcodebuild` (**BUILD SUCCEEDED**), and `./gradlew assembleDebug` (**BUILD SUCCESSFUL**)

### Feature 07: Structured Transcript & Dual-Mode UI (Chat ↔ Terminal)
- **Doc**: `docs/features/07-structured-transcript-dual-mode.md`
- **Scope**:
  - **Rust Core**: Streaming JSONL parser (`crates/core/src/transcript.rs`) for Google Antigravity and Claude Code transcripts, ANSI escape stripping, path traversal & sensitive directory protection (`validate_transcript_path`), remote detection and tailing (`detect_active_transcript_path`, `read_remote_transcript`).
  - **Platform (iOS)**: Dual-mode segmented toggle (`Terminal` ↔ `Chat UI`), `StructuredTranscriptView` with expandable thinking blocks and tool call cards, quick prompt composer bar.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-16-structured-transcript-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented & Verified with `cargo test` (40/40 passed), `swift test` (3/3 passed), and `xcodebuild` (**BUILD SUCCEEDED**)

### Feature 08: Smart Snippets & Fuzzy History Search
- **Doc**: `docs/features/08-smart-snippets-and-fuzzy-search.md`
- **Scope**:
  - **Rust Core**: Built-in developer snippet catalog (`default_developer_snippets`), subsequence fuzzy matcher (`fuzzy_search_snippets`), placeholder token parser (`extract_snippet_placeholders`), and secret-scrubbing remote shell history extractor (`extract_history_commands`, `fetch_remote_shell_history`).
  - **Platform (iOS)**: `[ ⚡ ]` accessory keyboard button, top bar bolt button, `SnippetsSheetView` with fuzzy search, category filter chips, interactive `{{placeholder}}` resolution sheet, custom snippet creation (`UserDefaults`), and remote shell history search.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-16-smart-snippets-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented & Verified with `cargo test` (40/40 passed), `swift test` (3/3 passed), and `xcodebuild` (**BUILD SUCCEEDED**)

### Feature 09: Remote Host Telemetry & Agent Process Monitor
- **Doc**: `docs/features/09-host-telemetry-and-process-monitor.md`
- **Scope**:
  - **Rust Core**: Single-shot POSIX telemetry script (`TELEMETRY_COLLECTION_SCRIPT`), cross-platform macOS (`sysctl`, `vm_stat`, `pmset -g batt`) & Linux (`/proc/loadavg`, `/proc/meminfo`) parser (`parse_host_telemetry`), process category classifier (`ProcessCategory`: `AiAgent`, `DevServer`, `System`), PID safety validator (`validate_kill_pid`), and `SshSessionHandle::fetch_host_telemetry` / `kill_remote_process`.
  - **Platform (iOS)**: Top bar gauge button (`gauge.with.dots.needle.33percent`), `HostTelemetrySheetView` with CPU/RAM/Disk/Battery/Ollama cards, process category filter chips, and confirmation dialog for `SIGTERM` / `SIGKILL`.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-16-host-telemetry-audit.md` — VERDICT: CLEAR)
- **Status**: Implemented & Verified with `cargo test` (44/44 passed), `swift test` (3/3 passed), and `xcodebuild` (**BUILD SUCCEEDED**)

### Feature 10: Remote File Browser & Quick Code Viewer
- **Doc**: `docs/features/10-remote-file-browser-and-viewer.md`
- **Scope**:
  - **Rust Core**: `crates/core/src/file_browser.rs` — tmux-aware `list_remote_directory`, `ls -la` parser, symlink-resolving path validator and credential blocklist, `read_remote_file_preview` (regular files, ≤128 KB, binary detection), `quote_remote_path_for_shell`.
  - **Platform (iOS)**: top bar `folder.fill` button, `FileBrowserSheetView` (drill-down, `..`/Home, hidden toggle, filter, `cd Here`) and `FilePreviewView` (line numbers, Copy, Insert Path).
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-16-remote-file-browser-audit.md` — 2 Medium + 2 Low found and fixed, no open issues)
- **Status**: Implemented & Verified with `cargo test` (53/53 passed, `file_browser.rs` coverage 96.6%), `cargo clippy -D warnings`, and `xcodebuild` (**BUILD SUCCEEDED**, device target). `make iphone` builds and signs; install on iPhone not yet confirmed (`devicectl` connection error, pending retry)

---

## Platform Parity

| Milestone | iOS | Android |
| :--- | :--- | :--- |
| M8 Web Preview | Done | Done (`WebPreviewDialog`) |
| M9 AI Assistant | Done | Done (`AiAssistantDialog`) |
| M10 Structured Transcript | Done | Done (`StructuredTranscriptScreen`) |
| M11 Snippets & History | Done | Not started |
| M12 Telemetry & Process Monitor | Done | Not started |
| M13 File Browser | Done | Not started |

## Backlog

- [ ] Confirm M13 on a physical iPhone (install + launch via `make iphone`).
- [ ] **Hook install lost-update window**: install is read → merge → atomic write; compare a content hash on the host before `mv` (audit `2026-10-16-agent-hooks-presets-audit.md` item 6).
- [x] **Host approval queue** (Feature 13 / M17, replaces the `control.sock` broker design): phone approvals work while the app is open.
- [ ] **Push wake-up for approvals** (APNs via the zero-knowledge Worker, Notification Service Extension, Live Activity; Feature 03 §4–5): until then approvals only reach the phone while the app is in the foreground; after 120 s the agent falls back to its native prompt.
- [ ] Approval long-poll (streaming exec) instead of 2 s polling; approvals for profiles with a custom hook directory.
- [ ] Codex (`hooks.json`, trust via `/hooks`) and Grok (`~/.grok/hooks`, fail-open) hook installers; verify the `grok` binary name.
- [ ] Verify Antigravity `run_command` arg key and hook exit-code semantics on the installed `agy`.
- [ ] Android UI for M11–M13 (needs an `android-ui-dev` skill; none exists in `.agents/skills/` yet) and a Kotlin bindings refresh.
- [ ] Hacker findings doc for Feature 04 (Android).
- [x] Integration test for exec-based methods against a loopback sshd (M14: telemetry + file browser; `crates/core/tests/loopback_sshd.rs`). History/transcript methods still uncovered.
- [ ] Re-verify the per-feature test counts above: Features 07 and 08 both report `cargo test` 40/40; the current suite is 53 tests.

### M14: Loopback sshd Integration Tests
- **Test**: `crates/core/tests/loopback_sshd.rs` — spawns its own `sshd` (ephemeral host key, `127.0.0.1`, ephemeral port, generated Ed25519 client key), connects `SshSessionHandle`, and exercises `fetch_host_telemetry`, `list_remote_directory` and `read_remote_file_preview` (hidden filter, binary detection, symlink-to-`.ssh` file and directory refused, missing file). Skips with a message if `sshd`/`ssh-keygen`/`tmux` are absent; cleans up sshd and its tmux session.
- **Status**: `cargo test` (53 unit + 1 integration passed), `cargo clippy --all-targets -D warnings` clean

### Feature 12 / M16: Toolbar Redesign, Server Popup & Host Ollama Check
- **Doc**: `docs/features/12-toolbar-redesign-and-host-checks.md`
- **Scope**: `AppAction` catalog + `ActionPanelView` (tap runs, long-press shows a hint, disabled-with-reason when offline), `ServerPickerPopup` replacing the confirmation dialog and the menu picker, `check_host_ollama` (`build_ollama_probe_command` / `parse_ollama_probe`, shared `hostprobe::watchdog_sh` with a descendant-tree kill that also works under dash) and the AI assistant states (checking / not installed / not running / no models / ready with host model list).
- **Status**: `cargo test` (87 unit + 8 hook + 4 loopback + 3 probe), clippy clean, `xcodebuild` BUILD SUCCEEDED; panel and popup layout checked offscreen. Not yet exercised on a device (popover chrome, long-press, haptics).

### Feature 11 / M15: Agent Approval Hooks & Launch Profiles
- **Doc**: `docs/features/11-agent-hooks-and-launch-profiles.md` (supersedes the hook details of Feature 03, which used a non-existent `beforeShellExecution` event and a hook that never blocked on deny)
- **Scope**:
  - **Hooks**: `scripts/agent-hooks/ssh-app-hook.py` (replaces `claude-hook.sh`); Claude Code `PermissionRequest`/`Notification`/`Stop` into `<CLAUDE_CONFIG_DIR>/settings.json`, Antigravity `PreToolUse(run_command)`/`Stop` into `~/.gemini/config/hooks.json`; idempotent, backup, symlink- and mode-preserving, refuses unparsable configs. Failure policy: defer to the agent's native policy, never auto-approve.
  - **Launch profiles**: `crates/core/src/agents.rs` compiles profiles to explicit `env VAR=… binary args; exec $SHELL -l` (aliases are not loaded by the app's login shell); validation rejects injection, `LD_*`/`DYLD_*`, secret-looking env keys.
  - **iOS**: `AgentProfilesSheetView` (profiles, auto-start choice for new tabs), tab auto-start via `addAgentSession`.
  - **Host shell environment**: per-profile `load_shell_rc` (`$SHELL -i -c`, loads `~/.zshrc`) and "Check on host" (`check_agent_environment`: login vs interactive resolution of the binary, host-side 8 s watchdog, 10 s client timeout).
  - **Hook presets & preview**: `HookPreset` (PhoneApproval / NotifyOnly / Custom command), `plan_agent_hooks`, `HookPreviewSheetView` shows installed vs planned entries and the resulting hooks section (never other settings) before **Install**; handler-level removal; duplicate-key and dangling-comment guards; exec output capped at 4 MB.
- **Researched, not implemented**: Codex and Grok hooks (see backlog).
- **Hacker Gate**: Completed twice: `docs/security-findings/2026-10-16-agent-hooks-audit.md` (1 Medium + 4 Low, fixed) and `docs/security-findings/2026-10-16-agent-hooks-presets-audit.md` (rc/probe/presets: 1 Medium + 6 Low/Info, all fixed and re-verified live except the documented read→write window)
- **Status**: `cargo test` (82 unit + 8 hook-script + 4 loopback sshd passed), `clippy --all-targets -D warnings` clean, `xcodebuild` (**BUILD SUCCEEDED**, device target). Not yet exercised on a physical iPhone or against a real `claude`/`agy` hook invocation.

### Feature 13 / M17: Host Approval Queue & Phone Approvals
- **Doc**: `docs/features/13-host-approval-broker.md` (supersedes the `control.sock` broker of Feature 03 §3 and the socket protocol of Feature 11 §4).
- **Scope**: `ssh-app-hook` is its own broker: a request is a `0600` file in `~/.ssh-app/run/pending`, the phone answers with an SSHSIG over `SSH_APP_APPROVAL_V1:{id}:{hash}:{nonce}:{host ts}:{approved}`, and the hook verifies it with `ssh-keygen -Y verify` against `~/.ssh-app/allowed_signers` using its own copy of the request. `ctl list|decide|ack|status` for the app; core: `list_pending_approvals`, `submit_approval_decision`, `acknowledge_notification`, `install_approval_key`, `approval_status`, `sign_approval_sshsig`; iOS: `ApprovalsCenter` (foreground polling per server), approval card with countdown, notices, profile status row.
- **Tests** (units only for the critical logic; the rest integration + e2e): 100 unit, 15 hook-script integration (real script + real `ssh-keygen`, forged/stale/misdirected decisions, FIFOs, floods, permissions), 5 loopback sshd incl. the end-to-end phone approval round trip (install → hook → list → sign → release for allow and deny), 6 probe tests; `approval_queue.rs` 98% line coverage.
- **Hacker**: `docs/security-findings/2026-10-16-approval-queue-audit.md` (FIFO blocking open, ambiguous digest; both fixed; signature fuzz 0/120 accepted).
- **Install**: the new script and the trusted key are written by "Review & install" on a profile (re-run it once after updating the app).
