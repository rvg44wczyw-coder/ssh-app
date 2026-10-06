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
| **M7: Android Client Architecture** | UniFFI Kotlin bindings, AndroidKeyStore Ed25519, Jetpack Compose, FCM/UnifiedPush | Planned (Spec Ready) | `doc-dev`, `rust-core-dev`, `android-ui-dev`, `rev`, `hacker` |

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
- **Hacker Gate**: Yes (AndroidKeyStore TEE/StrongBox, BiometricPrompt, Zero-Knowledge FCM / UnifiedPush)
- **Status**: Specification Written (Milestone M7)



