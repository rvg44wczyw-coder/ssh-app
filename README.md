# SSH-App (iOS + Rust Core)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-iOS%2017%2B%20%7C%20macOS-blue.svg)](ios/)
[![Rust Core](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](crates/core/)
[![Zero Battery Drain](https://img.shields.io/badge/Battery-Zero%20Drain%20(0%25%20idle)-brightgreen.svg)](#key-technical-features)
[![Zero Network Attack Surface](https://img.shields.io/badge/Security-0%20Listening%20Ports-success.svg)](#security--cryptography)

Native iOS mobile application (**SwiftUI + Rust Core via UniFFI**) for securely managing remote terminal sessions and autonomous AI coding agents (*Claude Code*, *Google Antigravity*, *Ollama / local models*, *OpenAI Codex*) on your MacBook / remote server over Tailscale.

---

## Architecture Overview

```text
┌────────────────────────────────────────────────────────────────────────┐
│                          SHARED RUST CORE                              │
│   crates/core (russh, ssh-key, ed25519-dalek, tmux, OutputThrottler)   │
└──────────────────┬─────────────────────────────────┬───────────────────┘
                   │ UniFFI Swift Bindings           │ UniFFI Kotlin (M7)
                   ▼                                 ▼
┌──────────────────────────────────────┐  ┌──────────────────────────────────────┐
│             iOS CLIENT               │  │           ANDROID CLIENT             │
├──────────────────────────────────────┤  ├──────────────────────────────────────┤
│ UI: SwiftUI                          │  │ UI: Jetpack Compose                  │
│ Terminal: SwiftTerm                  │  │ Terminal: Termux View / Compose PTY  │
│ Crypto: Keychain + Secure Enclave    │  │ Crypto: AndroidKeyStore (StrongBox)  │
│ Push: APNs via Cloudflare Worker     │  │ Push: FCM via Cloudflare Worker OR   │
│                                      │  │       UnifiedPush/ntfy via Tailscale │
│ Background: sceneDidEnterBackground  │  │ Background: Lifecycle ON_STOP        │
│ Snapshot Privacy: SwiftUI Overlay    │  │ Snapshot Privacy: Window FLAG_SECURE │
└──────────────────────────────────────┘  └──────────────────────────────────────┘
```

---

## Key Technical Features

### 1. Zero Battery Drain
- **Instant Socket Teardown**: When minimized (`sceneDidEnterBackground`), the TCP socket is immediately closed.
- **0% Idle CPU & Radio Usage**: The iPhone does not maintain background polling sockets.
- **Persistent Agent Execution**: The host persists long-running agent tasks safely inside `tmux`.
- **Instant Re-Attach**: On foregrounding (`didBecomeActive`), the Rust core instantaneously reconnects and re-attaches to the tmux session.

### 2. Zero Network Attack Surface on Host
- **No Open HTTP Ports**: Unlike alternative solutions that run unauthenticated local web servers on the Mac, `ssh-app` opens **ZERO** listening TCP/HTTP ports.
- **Local IPC**: Agent hooks communicate with the local approval daemon strictly through a Unix Domain Socket at `~/.ssh-app/run/control.sock` with file mode `0600` inside a directory restricted to `0700`.
- **Authenticated SSH**: Remote communication is strictly routed through authenticated SSH channels (`russh` with Ed25519 keys) over Tailscale WireGuard.

### 3. Hardware-Backed Cryptographic Approvals
- **Ed25519 Hardware Signatures**: Approvals are signed on-device using Ed25519 private keys stored in the iOS Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, `kSecAttrSynchronizable = false`).
- **Domain-Separated Canonical Payload**: Serialized as `SSH_APP_APPROVAL_V1:{id}:{command_hash_sha256}:{nonce}:{timestamp}:{approved}`.
- **Replay Protection**: Cryptographic freshness window of 120 seconds and single-use CSPRNG nonces.
- **Host Verification**: Verified on the MacBook against `~/.ssh/authorized_keys` before unblocking the agent.

### 4. Zero-Knowledge Wake-up Pings ($0/month)
- **Cloudflare Worker APNs Gateway**: A stateless Cloudflare Worker ([`scripts/apns-worker/worker.ts`](scripts/apns-worker/worker.ts)) forwards empty wake-up notifications (`content-available: 1`).
- **Total Privacy**: The worker receives **ONLY** an opaque 64-character hex device token. **Zero command text, zero file paths, and zero code snippets ever touch third-party cloud servers.**
- **Cost**: Runs completely on the Cloudflare Workers Free Tier (100,000 requests/day included, $0/month).

### 5. Developer Experience & UI
- **Terminal Rendering**: High-performance ANSI/VT100 rendering powered by SwiftTerm with TrueColor 24-bit (`xterm-256color:Tc`) support.
- **Interactive Approval Banner**: Floating banner showing agent badge, monospace command preview, SHA-256 hash, and haptic Approve/Deny buttons.
- **Native Git Diff Viewer**: Embedded modal sheet displaying syntax-highlighted unified diffs (`git diff HEAD`) directly fetched from the host.
- **Accessory Keyboard Bar**: Fixed top keyboard row with dedicated `✓ y⏎`, `✗ n⏎`, `Esc`, `Tab`, `Ctrl`, `|`, and arrow keys.
- **Privacy Masking**: Full-screen black privacy overlay with lock shield applied when the app is backgrounded, preventing sensitive terminal outputs from being captured in the iOS App Switcher.

---

## Project Structure

```text
ssh-app/
├── crates/
│   └── core/                # Pure Rust core (russh, ssh-key, tmux, UniFFI, ed25519-dalek)
├── packages/
│   └── SshCoreBridge/       # Swift Package / UniFFI generated bindings
├── ios/
│   ├── SSHApp.xcodeproj/    # Xcode project configuration
│   └── Sources/SSHApp/      # SwiftUI views, view models, and Keychain services
├── scripts/
│   ├── build-xcframework.sh # Multi-architecture XCFramework compilation script
│   ├── agent-hooks/         # Agent approval hooks (Claude Code, Antigravity)
│   └── apns-worker/         # Zero-Knowledge Cloudflare Worker APNs gateway
├── docs/
│   ├── features/            # Feature specifications (01 to 04)
│   ├── security-findings/   # Adversarial security audit reports (VERDICT: CLEAR)
│   └── roadmap.md           # Implementation milestones (M0 through M11)
├── .agents/skills/          # Dev-chain workflow skills
├── AGENTS.md                # Engineering constraints and architecture invariants
└── LICENSE                  # MIT License
```

---

## Getting Started

### Prerequisites
- macOS Sonoma or later
- **Rust**: 1.85+ with Apple targets:
  ```bash
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin
  ```
- **Xcode**: 15.0+ or 16.0+
- **Tailscale**: Installed and configured on your Mac and iPhone

### 1. Build Rust Core & XCFramework
Clone the repository and compile the multi-architecture framework:
```bash
git clone git@github.com:rvg44wczyw-coder/ssh-app.git
cd ssh-app
./scripts/build-xcframework.sh
```
This builds:
- `aarch64-apple-ios` (Physical iPhone / iPad)
- `aarch64-apple-ios-sim` (Apple Silicon iOS Simulator)
- `aarch64-apple-darwin` (macOS / SwiftUI Previews)
and writes `ssh_coreFFI.xcframework` into `packages/SshCoreBridge/Frameworks/`.

### 2. Run the iOS App
Open the project in Xcode:
```bash
open ios/SSHApp.xcodeproj
```
Select your target device or simulator and press **Cmd + R**.

### 3. Deploy the Zero-Knowledge Cloudflare Worker (Optional for Wake-ups)
```bash
cd scripts/apns-worker
cp wrangler.toml.example wrangler.toml
# Edit wrangler.toml with your Apple Developer Team ID & Key ID
npx wrangler secret put APPLE_P8_PRIVATE_KEY
npx wrangler deploy
```

### 4. Enable Claude Code Approval Hook
In your Mac's `~/.claude/settings.json`:
```json
{
  "hooks": {
    "PreToolUse": "/path/to/ssh-app/scripts/agent-hooks/claude-hook.sh"
  }
}
```

---

## Roadmap

| Milestone | Scope | Status |
| :--- | :--- | :---: |
| **M0–M5** | Core SSH engine, tmux multi-tab management, iOS UI, Zero Battery Drain | **Completed** |
| **M6** | Cryptographic approvals, Zero-Knowledge APNs, Diff viewer, Hacker audit | **Completed** |
| **M7** | Android Client (Jetpack Compose, AndroidKeyStore StrongBox, FCM / UnifiedPush) | **Spec Ready** |
| **M8** | In-App Web Preview & SSH Port Forwarding (`localhost:3000` via tunnel) | **Planned** |
| **M9** | AI Command Prompt & Natural Language Shell Assistant | **Planned** |
| **M10** | Structured Transcript Viewer & Dual-Mode UI (Chat $\leftrightarrow$ Terminal) | **Planned** |
| **M11** | Smart Snippets & Fuzzy History Search | **Planned** |

See [`docs/roadmap.md`](docs/roadmap.md) for detailed descriptions.

---

## Security & Audits

Every milestone touching networking, cryptography, or IPC must pass the **Hacker** adversarial audit gate.  
All audit reports are available in [`docs/security-findings/`](docs/security-findings/):
- **2026-10-06**: Agent Control & Secure Approvals Audit — **VERDICT: CLEAR**

---

## License

This project is licensed under the terms of the [MIT License](LICENSE).
