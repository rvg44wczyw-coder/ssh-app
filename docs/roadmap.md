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
| **M18: Agent Permissions & Hook Settings** | Per-profile editor for Claude Code `permissions.allow/ask/deny/defaultMode` with validation, templates, warnings, preview and guarded apply (only those four fields touched, other settings preserved); configurable approval-hook matcher and events | Completed (Rust + iOS; live phone test pending) | `doc-dev`, `rev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `rev`, `ios-ui-dev`, `rev`, `hacker` |
| **M19: Terminal Input, Session Permissions & Appearance** | tmux mouse-wheel scroll and tap-as-click, keyboard quick commands, local Ollama chat, session permissions menu with suggestions and CLAUDE_CONFIG_DIR detection, permission mode per launch profile, accent colors (presets + custom), terminal color schemes (built-in + editor, per agent/profile), profile and app icons (see `docs/features/17-…`) | Completed (iOS + Rust; on-device checks pending, `rev`/`hacker` pending) | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev` |
| **M20: Payments (StoreKit 2)** | Pro subscription (weekly / monthly / yearly with trials) and lifetime purchase, free tier of 20 sessions and 3 servers, paywall with Restore Purchases, optional via the `PAYMENTS` build option (see `docs/features/18-payments.md`) | Code merged, **switched off** (no `PAYMENTS` flag; the app is free for now). To do before enabling: prices and App Store Connect setup, privacy/terms URLs, `rev`/`hacker` | `doc-dev`, `ios-ui-dev` |
| **M21: Hook templates & host guard** | 33 hook templates, host-side deny/ask/allow rules, context and stop message, command analysis on the approval card, custom-command lint, drift warning, session registry (see `docs/features/19-hooks-templates-and-host-guard.md`) | Implemented (Rust + iOS; Codex/Grok and live-Claude checks pending; `rev` and `hacker` done: `docs/security-findings/2026-10-18-m19-m21-review.md`; earlier self-audit `2026-10-17-host-guard.md`) | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker` |
| **M27: Multimodal Agent Uploads** | Camera/photo library screenshot pipeline, client EXIF-stripping, sandboxed `~/.ssh-app/run/uploads` transport, agent `@path` injection (`docs/features/38-multimodal-agent-uploads.md`) | Spec written | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M28: Live Activities & Dynamic Island** | ActivityKit monitoring for long-running agent tasks (tests/refactors), APNs push updates, Lock Screen Abort/View controls (`docs/features/39-live-activities-and-dynamic-island.md`) | Spec written | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M29: Agent Voice Prompt Dictation** | 100% on-device speech recognition, technical syntax normalizer (`auth.rs`, `--flags`, `camelCase`), zero audio leakage (`docs/features/40-agent-voice-dictation.md`) | Spec written | `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M30: Visual Artifacts & Diagrams** | Mermaid flowchart/sequence/state diagram renderer, sandboxed CSP WebView, SVG zoom/export, unified split diff viewer (`docs/features/41-agent-artifacts-and-visualizer.md`) | Spec written | `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M31: iPadOS Productivity & Keyboards** | NavigationSplitView 3-column layout, Stage Manager multi-window, UIKeyCommand hardware shortcuts (`Cmd+T`, `Ctrl+C`), external display mode (`docs/features/42-ipados-hardware-keyboard-and-stage-manager.md`) | Spec written | `doc-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M32: Hardware Keys & iCloud Vault Sync** | FIDO2/YubiKey (`ed25519-sk`), OpenSSH private key import with zeroization, zero-knowledge iCloud Keychain host sync (`docs/features/43-hardware-keys-and-vault-sync.md`) | Spec written | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M33: Zero-Touch Host Onboarding** | paste-one-line or password pairing (no hosted script), automatic helper deploy, safe atomic `authorized_keys` append (`ssh-app-host pair`), read-only host doctor (`docs/features/44-zero-touch-host-onboarding.md`) | Implemented (helper, core, app; device run pending) | `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker` |
| **M34: Docker Containers & In-Container Terminal** | Container monitoring (green/red status, RAM, CPU), 1-tap `[ ↺ Restart ]`, `[ ⎘ Logs ]`, and 1-tap `[ >_ Terminal ]` spawn directly into container shell (`docs/features/45-docker-container-manager-and-terminal.md`) | Implemented over the docker CLI (list, stats, start/stop/restart, logs, container terminal tab); helper path and Shortcuts intent later; device run pending | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M35: Modular Configuration Engine** | Universal configuration architecture with granular tuning across all 9 subsystems (Docker, Uploads, Live Activities, Voice, Mermaid, Keyboard, Vault, Mesh, SSH Core) + JSON Export/Import (`docs/features/46-modular-configuration-and-power-settings.md`) | Spec written | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M37: Apple Shortcuts & App Intents Automation** | Siri voice commands (*"Hey Siri, run tests with Claude"*), iOS 18 Control Center widgets, Lock Screen actions, and Shortcuts parametric workflows (`docs/features/48-apple-shortcuts-and-app-intents.md`) | First release implemented (host health, saved command, agent task for Claude profiles); Docker and widgets later; device run pending | `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M38: Zellij Terminal Multiplexer Support** | Zellij as a third `MultiplexerKind` (attach, detach, kill, list only; no feed) (auto-detect, session attach/create/kill, KDL layouts) for modern Rust/Go environments (`docs/features/49-zellij-multiplexer-support.md`) | Milestone A implemented (attach, list, kill; feed parity is milestone B, not scheduled); device run pending | `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker` |
| **M39: Auto-approval by command category** | Command categories (read-only, git, Rust, Python, Go, Node, Swift, make, Docker, network, installs) with Ask/Auto/Block per agent, presets (Cautious, Developer, Trusted project, Lockdown), projects with a hard boundary, temporary trust, audit log; compiled into the host guard (`docs/features/50-auto-approval-categories.md`) | Implemented (core, hook script + Rust port, signed temporary trust, iOS screens, card trust button); `rev` and security review done (§12-13 of the spec); device run pending | `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker` |

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
- [x] **Hook install lost-update window**: the config write now compares a SHA-256 of the file on the host with what was read and refuses (`CONFIG_CHANGED`) if it differs; a failed write is no longer reported as success. Applies to hooks and permissions.
- [x] **Host approval queue** (Feature 13 / M17, replaces the `control.sock` broker design): phone approvals work while the app is open.
- [ ] **Push wake-up for approvals** (APNs via the zero-knowledge Worker, Notification Service Extension, Live Activity; Feature 03 §4–5): until then approvals only reach the phone while the app is in the foreground; after 120 s the agent falls back to its native prompt.
- [x] Approval long-poll: `ctl wait <known ids>` holds the call up to 20 s and answers the moment the pending set changes (`wait_pending_approvals`); used with a single host, falls back to the 2 s poll for several hosts or an older hook script. Tests: `ctl_wait_returns_on_change_and_holds_otherwise`. Not run on a device yet.
- [x] Approvals for profiles with a custom hook directory: not applicable, the app never sets a custom hook directory (every call passes `nil`); a core caller that does would need `ctl` from its own path.
- [x] Codex and Grok hook installers, from the official docs (Codex 0.163.0 installed for a format check; its `doctor` does not validate hooks and a live run needs OpenAI auth, so neither agent was exercised live). Codex: `$CODEX_HOME|~/.codex/hooks.json`, `hooks` key, `PermissionRequest` with Claude's reply shape, trust by hash via `/hooks` (shown in the preview). Grok: own file `~/.grok/hooks/ssh-app.json`, `PreToolUse` (default matcher `Bash`) can only block, so only a phone Deny acts; fail-open on its side. Still open: verify the `grok` binary name and run both agents live.
- [x] Verified Antigravity hooks on the installed `agy` 1.4.0 (docs shipped in `~/.gemini/antigravity-cli/builtin/skills/agy-customizations/docs/hooks.md` plus a live probe): tool `run_command`, arg `CommandLine`, `workspacePaths`; decisions are `allow` / `deny` / `ask` / `force_ask` (`allow` auto-approves; `auto_approve` does not exist); `Stop` (and other matcher-less events) are flat handler lists. Fixed: the no-opinion fallback was `allow` (auto-approved on broker failure) and approvals used the invalid `auto_approve`; `Stop` was written in the grouped form. Exit-code semantics are not needed: the contract is JSON on stdout.
- [ ] **Host-side service** (analysis: `docs/features/16-host-service-analysis.md`): P0 typed `ssh-app-host` helper replacing embedded Python/shell, P1 hook + stdio rpc, P2 launchd daemon (agent state, usage index), P3 push, P4 host rule engine, P5 history/widget. Waiting for the owner's decisions in §9. **Owner decisions taken 2026-10-18** (`docs/features/16-host-service-analysis.md` §11): agent only while a phone session exists, APNs relay, macOS only, embed in the app first and Homebrew later. P0 (`docs/features/24-host-helper.md`) implemented: typed helper with usage, embedded in the core, pinned install, Python fallback; not run on a device. P1 (`docs/features/25-host-hook.md`) implemented: `hook`, `ctl`, `statusline` in the helper behind a pinned-digest shim in the installed hook script; the 32 hook tests pass on both implementations, guard differential fuzz vs Python; not run on a device. P3 (push) implemented T1-T3 (`docs/features/35-push-relay.md`): Worker deployed on the owner's Cloudflare, host `push-*` commands and sink, core calls, app card; device run (T4) pending. P2 (`docs/features/26-host-agent.md`) implemented in the helper: lease-based agent started by `rpc` (no launchd), private socket, derived agent state, event sink for push; app wiring done (`host_agent_state`, Agent row in the Host service card, loopback-tested); `tmux -C` and the usage index still open. Next: P3 when the Apple key and Cloudflare account exist.
- [x] **Project view and git (`docs/features/28-session-project-view.md`)**: G0 coloured file list, G1 branches, G2 worktrees (sibling folder, open in a tab), G3 cherry-pick / revert / reset (soft, mixed) / reword, G4 squash top N / drop / merge / rebase / stash, G5 stage, commit, fetch, pull (ff-only), push (never forced). Core + sheets done with loopback tests on temporary repositories and a security pass (`docs/security-findings/2026-10-19-git-branches-g1.md`); every write has a confirmation and a backup ref. Hunk staging done (G5b, swipe on a modified file in Changes; device check pending). Reword of older commits done (G4b, `docs/security-findings/2026-10-19-git-reword-g4b.md`). Not done: the Project sheet is now one tabbed sheet (Changes, Log, Branches; `ProjectSheetView`). Device check pending.
- [ ] **Context menus on quick commands** (`docs/features/30-command-context-menus.md`): research done (agy has `agy models`, Claude has aliases and an internal catalog cache, Ollama has `ollama list`); implemented (helper `models`, `fetch_agent_models`, long-press menu, editor); agy `/model` argument syntax unverified.
- [ ] **TODO after the cleanup (2026-10-18)**. Done: `MainTerminalView` sheets split, `agent_hooks` and `permissions` split, live agent events (`watch_host_agent`). Agent: tmux-detected sessions now produce live events while subscribed (`docs/features/27-agent-tmux-control.md`; a `tmux -C` client was rejected, see there). Left:
  - `MainTerminalView.swift` split further (2026-10-19): `ActiveTerminalSessionView.swift`, `MainTerminalAssistant.swift` (jump menu, assistant routing); 1038 → 701 lines. Lifecycle/approval polling still in the main file.
  - Remove the Python hook fallback, only after device runs of the helper path (the app must keep working on hosts without the helper).
  - Telemetry and the file browser through the helper (`docs/features/32-helper-telemetry-and-files.md`): T1–T3 implemented and audited (`telemetry`, `ls`, `read`, `kill`; shell paths stay as fallback), the app passes the pin everywhere; T4 is the device run, waits for the owner's "собери".
  - P3 push: T1-T3 done (Worker deployed, host, core, app); T4 on the iPhone pending.
  - Linux and Windows (WSL2) hosts: spec `docs/features/33-host-os-support.md` (phases H1–H5, owner questions in §11): H1–H4 implemented and audited (Linux musl helpers embedded, WSL2 snippets, host-aware pin; simulator build only, not run on a Linux/WSL host through the app), H5 native Windows: spec `docs/features/34-native-windows-host.md` (W1–W7, started on the owner's word 2026-10-19); Watch registration and Developer Mode; Files extension on a device; live hook checks of Qwen, Crush, OpenCode, Codex, Grok and `ollama launch`; Files extras (Face ID per server, favourites). Android stays excluded.
- [x] Hooks hardening from `docs/features/15-hooks-capability-analysis.md` §5 items 1–5 were delivered with M21 (protect-agent-config template, command analysis on the approval card, custom-command lint, drift warning, host guard rules, session registry). Only §5 item 6 stays open: Codex and Grok are not installed on this machine, so their hooks cannot be run live.
- [x] Agent feed (`docs/features/31-agent-feed.md`): Chat mode rebuilt as one selectable text view in Claude Code's look, per-tab transcript lookup, live tail, find, diffs for Edit/Write, formatted Markdown. Device check pending.
- [ ] Terminal: scroll/tap-as-click/quick-command bar confirmed on the phone.
- [ ] Android UI for M11–M13 (needs an `android-ui-dev` skill; none exists in `.agents/skills/` yet) and a Kotlin bindings refresh.
- [ ] Hacker findings doc for Feature 04 (Android).
- [x] Integration test for exec-based methods against a loopback sshd (M14: telemetry + file browser; `crates/core/tests/loopback_sshd.rs`). History/transcript methods still uncovered.
- [x] Loopback tests for history and transcript exist (`history_transcript_kill_and_probes_against_loopback_sshd`, `transcript_detection_runs_through_the_login_shell_with_its_argument`).
- [x] Per-feature test counts above are snapshots from when each feature landed (they are not re-run). Current `cargo test -p ssh-core` (2026-10-18): 191 lib + 32 hook_script + 22 loopback_sshd + 6 ollama_probe + 3 opencode_plugin = 254 passed.

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

### Fix: Git Diff screen showed git usage text
- **Cause**: the screen ran `git diff HEAD` on a fresh SSH exec channel, which starts in `$HOME`; outside a repository git falls back to `--no-index` and prints its usage text.
- **Fix**: `fetch_git_diff(session_name)` asks tmux for the pane's current directory and runs `git -C <repo root> diff HEAD` there (`--no-ext-diff --no-textconv`, `core.fsmonitor=false`, 1 MB cap, 20 s host watchdog; repos without a commit show staged changes; untracked files listed). The screen uses the live tab session and shows repo, branch, untracked files and clear states for "not a git repository" / "session not found".
- **Tests**: 5 unit (marker spoofing, caps, usage text not mistaken for a diff) and a loopback sshd end-to-end test with real git + tmux (changed repo, non-repo, no-commit repo, hostile directory name, missing session).

### M22 (planned): Files app integration, `docs/features/21-files-provider.md`
- File Provider extension: servers as locations in the Files app. **Stages 1-2 implemented** (read, download, upload, create folder, rename/move, delete of files and empty folders), hacker pass done (`docs/security-findings/2026-10-17-files-provider.md`). Open: device test, Linux `stat` run, per-server Face ID gate, favourites.

### M23: Apple Watch companion for approvals (stage 1 implemented, stage 2 push pending)
- Goal: approve / deny agent requests (and see notices) from the watch. Phone-relay design: the SSHSIG is still produced on the iPhone (key stays in its Keychain); the watch shows the request (tool, command tail, analysis flags, countdown) and sends allow/deny to the phone via WatchConnectivity (`sendMessage` wakes the phone app); notification actions mirrored from the phone give the same buttons. A watch-held key or a direct SSH connection from the watch is rejected: the watch cannot reach the tailnet by itself and a second signing key would widen the trust list on the host.
- Spec: `docs/features/22-watch-approvals.md` (delivery without push only works while the phone app is foreground; push channel awaits the owner's decision). Open: watchOS target in the hand-maintained `project.pbxproj`, behaviour when the phone is unreachable (show "open iPhone", never auto-approve), haptics, hacker gate done (`docs/security-findings/2026-10-18-watch-bridge.md`, clean, code analysis only).

- **M23 status**: stage 1 built (watchOS app + `WatchBridge`, `docs/features/22-watch-approvals.md` section 10). Needs a real watch to verify; push wake-up (A/B in the spec) is the owner's decision.

### M24: Local-model agents and assistant card (`docs/features/23-local-models.md`)
- **Stage 1 implemented**: OpenCode, Qwen Code, Crush (phone approvals), Cline, Aider, Goose, Ollama chat (launch only). Hook behaviour of the three approval-capable tools is from their docs and fake-driven tests, not run live.
- **Stage 2 implemented** (assistant as a card per server, chat state in `AssistantStore`, in memory; see §7 of the spec). UI not run on a device.

### Feature 36 / M25: Agent Feed Presentation Styles (`docs/features/36-agent-feed-styles.md`)
- **Doc**: `docs/features/36-agent-feed-styles.md`
- **Scope**: User-switchable feed styling in Settings (`Pro Terminal`, `Native Assistant`, `Dev Studio`), strict no-emoji policy (SF Symbols & clean typography only), adaptive attributed text layout in `FeedTextView`, and responsive prompt cards.
- **Roles**: `doc-dev`, `ios-ui-dev`, `rev`
- **Status**: Implemented in `ios/` (2026-10-20): three styles, git header with shortened long names, Dev Studio pipeline checklist, emoji audit; unit tests pass. Not yet checked on a device or reviewed by `rev`.

### Feature 38 / M27: Multimodal Agent Uploads & Mobile Screenshot Pipeline (`docs/features/38-multimodal-agent-uploads.md`)
- **Doc**: `docs/features/38-multimodal-agent-uploads.md`
- **Scope**: Native photo library/camera/clipboard picker, client-side EXIF stripping & HEIC->PNG conversion, sandboxed upload into `~/.ssh-app/run/uploads/<sha256>.<ext>`, TTL prune daemon, agent `@path` injection (Claude Code / agy / Ollama).
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 39 / M28: Live Activities & Dynamic Island for Long-Running Agent Tasks (`docs/features/39-live-activities-and-dynamic-island.md`)
- **Doc**: `docs/features/39-live-activities-and-dynamic-island.md`
- **Scope**: ActivityKit Dynamic Island (Compact, Minimal, Expanded) & Lock Screen live status, zero-battery APNs push relay updates, interactive Abort / Open Session actions, strictly SF Symbols without emojis.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 40 / M29: Voice Input & Technical Agent Prompt Dictation (`docs/features/40-agent-voice-dictation.md`)
- **Doc**: `docs/features/40-agent-voice-dictation.md`
- **Scope**: 100% on-device speech recognition via Apple Speech (`requiresOnDeviceRecognition`), technical syntax normalizer (`auth.rs`, `--flags`, `camelCase`), prompt action chips, zero audio leakage.
- **Roles**: `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 41 / M30: Agent Artifacts & Interactive Architecture Visualizer (`docs/features/41-agent-artifacts-and-visualizer.md`)
- **Doc**: `docs/features/41-agent-artifacts-and-visualizer.md`
- **Scope**: Automatic extraction of Mermaid diagrams and markdown reports from agent feed, hardened sandboxed WKWebView (CSP `default-src 'none'`), SVG pinch/zoom/pan, dark-mode matching, PNG/SVG export, unified diff viewer.
- **Roles**: `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 42 / M31: iPadOS Native Experience, External Hardware Keyboard & Multi-Window (`docs/features/42-ipados-hardware-keyboard-and-stage-manager.md`)
- **Doc**: `docs/features/42-ipados-hardware-keyboard-and-stage-manager.md`
- **Scope**: 3-column NavigationSplitView, Stage Manager concurrent sessions, full UIKeyCommand hardware keyboard shortcuts (`Cmd+T`, `Cmd+W`, `Ctrl+C`, `Cmd+1..9`), external monitor full-screen PTY projection, pointer hover effects.
- **Roles**: `doc-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 43 / M32: Hardware Security Keys (FIDO2 / YubiKey), Key Import & iCloud Keychain Sync (`docs/features/43-hardware-keys-and-vault-sync.md`)
- **Doc**: `docs/features/43-hardware-keys-and-vault-sync.md`
- **Scope**: OpenSSH FIDO2 (`ed25519-sk`, `ecdsa-sk`) via NFC / USB-C YubiKey, OpenSSH private key import with Argon2 / zeroization, end-to-end encrypted iCloud Keychain sync of server profiles (zero-knowledge: private keys never leave local device).
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 44 / M33: Zero-Touch Host Onboarding & Instant Pairing (`docs/features/44-zero-touch-host-onboarding.md`)
- **Doc**: `docs/features/44-zero-touch-host-onboarding.md`
- **Scope**: pair a new host without typing into `authorized_keys`: a one-time password login that adds the phone's key after the host key is shown and pinned, or a pasted command; `ssh-app-host pair` (safe append) and `ssh-app-host doctor` (read-only facts) in the helper; a "Host check" with suggestions in the app. No hosted script, no QR/rendezvous.
- **Roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Completed (`docs/security-findings/2026-10-20-onboarding-pair-doctor.md`)
- **Status**: Implemented (2026-10-20); Rust, bridge and app build and test green in the simulator; not run on a phone yet. Open: one continuous wizard, colour blocks for the fingerprint.

### Feature 45 / M34: Docker Container Manager & In-Container Terminal Sessions (`docs/features/45-docker-container-manager-and-terminal.md`)
- **Doc**: `docs/features/45-docker-container-manager-and-terminal.md`
- **Scope**: Docker container list with green/red status indicators, memory & CPU statistics, 1-tap `[ ↺ Restart ]`, slide-over `[ ⎘ Logs ]` viewer, and 1-tap `[ >_ Terminal ]` opening an interactive shell tab directly inside the container via `docker exec -it`.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev`, 2026-10-20; binding corrections at the end of the spec), implementation pending.

### Feature 46 / M35: Modular Configuration Engine & Granular Subsystem Settings (`docs/features/46-modular-configuration-and-power-settings.md`)
- **Doc**: `docs/features/46-modular-configuration-and-power-settings.md`
- **Scope**: Universal configuration engine in Rust Core & Swift SettingsStore: granular controls for Docker (custom sockets, shells, log bounds), Uploads (compression, dimensions, TTL, EXIF), Live Activities (throttling, Face ID), Voice (locales, custom lexicon), Mermaid (themes, line caps), Keyboard (remapping, debounce), Vault (sync toggles), Mesh (turn limits, leases), and Core SSH (grace periods, timeouts, TrueColor). Clean JSON Export/Import and per-module Reset.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev` + `hacker`, 2026-10-20), implementation pending.

### Feature 48 / M37: Apple Shortcuts & App Intents Automation (`docs/features/48-apple-shortcuts-and-app-intents.md`)
- **Doc**: `docs/features/48-apple-shortcuts-and-app-intents.md`
- **Scope**: App Intents framework integration for Siri voice commands (*"Hey Siri, run tests with Claude on MacBook"*), iOS 18 Control Center widgets, Lock Screen triggers, and Shortcuts parametric actions with structured JSON outputs.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev` + `hacker`, 2026-10-20), implementation pending.

### Feature 49 / M38: Zellij Terminal Multiplexer Support (`docs/features/49-zellij-multiplexer-support.md`)
- **Doc**: `docs/features/49-zellij-multiplexer-support.md`
- **Scope**: Universal `MultiplexerKind` trait supporting both `tmux` and `zellij` (auto-detect, session attach/create/kill, KDL layouts) for modern Rust/Go environments, dynamic PTY resizing, and native TrueColor.
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Status**: Spec reviewed and corrected (`rev` + `hacker`, 2026-10-20), implementation pending. Scope cut to attach/detach only (milestone A).

### Feature 50 / M39: Auto-approval by command category (`docs/features/50-auto-approval-categories.md`)
- **Doc**: `docs/features/50-auto-approval-categories.md`
- **Scope**: categories with levels per agent, presets, trusted folders for commands that run project code, audit log; enforced by the existing host guard (feature 19).
- **Roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Status**: Implemented (2026-10-21): categories, presets, per-agent and per-project policy, boundary check, audit log, signed temporary trust, iOS screens and trust menu on the approval card; reviewed (spec §12-13). Device run pending.

## Plan limits (2026-10-18)
- Claude: real 5h/7d percentages come only from the `statusLine` JSON (`rate_limits`). The hook script has a `claude statusline` mode (wrapper that records them under `~/.ssh-app/run/limits/<hash of CLAUDE_CONFIG_DIR>.json` and runs the user's previous status line); installed per profile from the Hooks screen ("Plan limits"), so the personal profile (`~/.claude-personal`) is covered by its own config dir. The usage probe shows them in the strip (`5h 42% 7d 18%`).
- Codex: read from its own session files (`token_count.rate_limits`). Format taken from memory/docs, not verified on a live Codex.
- Antigravity, Grok: no source found; model and context only.
