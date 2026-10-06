# Security Audit: Agent Control & Secure Approvals (Milestone M6 / Feature 03)

**Date:** 2026-10-06  
**Auditor:** Hacker (Adversarial Security Auditor)  
**Target:** Cryptographic approval engine, Cloudflare Worker APNs wake-up gateway, agent CLI hooks, Diff viewer, and iOS approval UI.

---

## 1. Executive Summary

An adversarial security audit was performed on the Agent Control and Secure Approvals feature set (Feature 03).
The objective was to evaluate the architecture against malicious command spoofing, MITM injection, remote host exposure, local privilege escalation, cryptographic replay attacks, ANSI escape reflection, and sensitive data leakage.

During the audit, one **Medium Severity** information leakage finding was identified and remediated immediately:
- **[REMEDIATED] VULN-01 (Medium): Raw Keystrokes Dumped to Console Logs**: Terminal input handling routines in Rust (`crates/core/src/session.rs`) and Swift (`TerminalSessionViewModel.swift`, `TerminalContainerView.swift`) logged raw user keystroke bytes. This could expose passwords or sensitive tokens in device logs. Keystroke byte dumping has been replaced with length-only telemetry.

All core cryptographic guarantees, zero-listening-port invariants, and zero-knowledge privacy boundaries were verified and confirmed sound.

---

## 2. Threat Vector Analysis & Audit Details

### Vector 1: Host Network Exposure & Remote Attack Surface
- **Analysis**: Typical competitor architectures often expose unauthenticated or token-authenticated HTTP/REST servers on host loopback or LAN ports, creating cross-site scripting (DNS rebinding) and local port hijacking vectors.
- **Verification**: `ssh-app` introduces **ZERO** open TCP/HTTP listening sockets on the MacBook host.
- **IPC Mechanism**: Local CLI agent hooks (e.g. Claude Code `PreToolUse`, Antigravity `beforeShellExecution`) communicate exclusively via a local Unix Domain Socket at `~/.ssh-app/run/control.sock`.
- **Filesystem Permissions**: The socket directory is enforced at `0700` (`drwx------`) and socket at `0600` (`srw-------`), preventing unauthorized local users from intercepting or injecting approval requests.

### Vector 2: Approval Cryptography & Replay Resistance
- **Payload Structure**: Evaluated `CanonicalSigningPayload` serialization:
  `SSH_APP_APPROVAL_V1:{id}:{command_hash_sha256}:{nonce}:{timestamp_sec}:{approved}`
- **Integrity**: Command text is hashed via SHA-256 (`hash_command_sha256`), preventing payload manipulation, canonical boundary attacks, or length-extension vulnerabilities.
- **Freshness Window**: Evaluated `verify_approval_freshness(timestamp_sec, current_time_sec, max_drift_sec)`. Approvals older than 120 seconds or timestamps from beyond the clock drift tolerance are strictly rejected.
- **Cryptographic Signatures**: Evaluated `verify_approval_signature`. Signatures are generated using Ed25519 private keys stored in the iOS Keychain and verified using `ed25519-dalek` v3 against OpenSSH public keys (`ssh-ed25519`). Signatures not strictly 64 bytes or failing Curve25519 group verification are rejected.

### Vector 3: Zero-Knowledge APNs Wake-up Gateway
- **Privacy Boundary**: Evaluated `scripts/apns-worker/worker.ts` deployed on Cloudflare Workers.
- **Payload Inspection**: The worker receives **ONLY** an opaque 64-character hex device token (`^[a-fA-F0-9]{64}$`). No command strings, usernames, IP addresses, or file paths are ever transmitted through the worker or Apple APNs.
- **SSRF & Injection Resistance**: Strict regex matching on `device_token` prevents path traversal and header injection into the APNs HTTP/2 endpoint (`https://api.push.apple.com/3/device/${deviceToken}`).
- **Data Flow**: When an agent requests approval while iOS is backgrounded, the host triggers the Cloudflare worker with the device token; iOS wakes up for an ephemeral background task, reconnects directly to the host via authenticated SSH over Tailscale, and retrieves the pending approval details privately.

### Vector 4: Information Leakage & Data-at-Rest Protection
- **Audit Finding [VULN-01 - REMEDIATED]**:
  - *Location*: `session.rs` line 92, `TerminalSessionViewModel.swift` line 74, `TerminalContainerView.swift` line 362.
  - *Issue*: Logging output printed raw byte arrays of terminal user input (`bytes=\([UInt8](data))`).
  - *Remediation*: Removed raw byte arrays from all three files; updated logging to report byte length only (`len=\(data.count)`).
- **Keychain Security**:
  - Private keys are stored in iOS Keychain with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and `kSecAttrSynchronizable = false`. Keys are never synchronized to iCloud or included in unencrypted backups.
  - In Rust core, `ssh-key::PrivateKey` implements `ZeroizeOnDrop`, ensuring key material is wiped from heap memory upon deallocation.
- **Background Snapshot Privacy**:
  - `MainTerminalView.swift` applies an opaque background privacy overlay with `Color.black` and a lock shield icon whenever `scenePhase == .background`. This prevents the iOS multitasking app switcher from capturing cached screenshots containing sensitive terminal outputs or credentials.

### Vector 5: ANSI Escape Injection & Terminal Exploitation
- **Terminal Reflection Protections**: `TerminalContainerView.swift` inspects terminal output and blocks synthetic terminal responses (`\033P>|`, `\033[?65`, `\033[>65`, window resize status queries) from being reflected into the remote shell.
- **Terminal Throttling**: `OutputThrottler` coalesces output batches (max 64KB, 16ms intervals), protecting the UI thread and SwiftTerm rendering engine against DoS and memory exhaustion attacks from unbounded agent log streams.

---

## 3. Findings & Remediations Log

| ID | Severity | Status | Description | Remediation |
|---|---|---|---|---|
| **VULN-01** | Medium | **Fixed** | Raw terminal input bytes logged to console output | Stripped raw bytes; replaced with length-only logging |
| **OBS-01** | Low / Info | **Documented** | SSH host key verification is delegated to Tailscale WireGuard mesh | Verified acceptable in private Tailnet topology; recommend host key pinning (`known_hosts`) for open WAN connections |

---

## 4. Security Audit Verdict

```text
============================================================
              SECURITY AUDIT VERDICT: CLEAR
============================================================
All identified items remediated and verified.
Rust Core, UniFFI bridge, and iOS UI compile and pass all tests.
No unauthenticated listening ports or plaintext cloud exposures.
============================================================
```
