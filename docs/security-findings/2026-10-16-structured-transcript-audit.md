# Security Audit Report: Structured Transcript & Dual-Mode UI (Feature 07 / Milestone M10)

**Date:** 2026-10-16  
**Auditor:** `hacker` (Adversarial Security Auditor)  
**Target Modules:**
- `crates/core/src/transcript.rs`
- `crates/core/src/session.rs` (`read_remote_transcript`, `detect_active_transcript_path`)
- `ios/Sources/SSHApp/Views/StructuredTranscriptView.swift`
- `ios/Sources/SSHApp/ViewModels/TerminalSessionViewModel.swift`

---

## 1. Executive Summary

An adversarial security audit was conducted on the Structured Transcript & Dual-Mode UI implementation. During the review, three potential attack vectors in remote path validation and shell script parameterization were identified and remediated prior to sign-off:

1. **Path Traversal & Arbitrary Sensitive File Read (`..` & Substring Bypass)**:
   - *Initial Risk*: `validate_transcript_path` checked prefixes (`~/.ssh/`, `/etc/`) but did not reject `..` traversal sequences or absolute paths containing `/.ssh/` (e.g., `/Users/user/.ssh/id_ed25519`).
   - *Remediation*: Added strict rejection of `..`, leading `-` flags, control characters, and sensitive path segments (`/.ssh/`, `/.gnupg/`, `/.aws/`, `/Library/Keychains/`).
2. **Command / Option Injection in `tail` and `detect_active_transcript_path`**:
   - *Initial Risk*: `base_dir` was formatted into a single-quoted `zsh -l -c` block, and `tail -n 500` did not include `--` before the path argument.
   - *Remediation*: `base_dir` is now validated via `validate_transcript_path`, quoted with `shell_quote`, and passed as a positional parameter (`$1`) after `--`. `tail` now uses `tail -n 500 -- <quoted_path>`.
3. **ANSI Escape & Terminal Spoofing Prevention**:
   - *Verification*: All untrusted transcript content, thinking blocks, and tool call results are sanitized through `strip_ansi_codes` before leaving the Rust core.
4. **Denial-of-Service (OOM) Protection**:
   - *Verification*: Remote reads are bounded to `tail -n 500`, and `parse_transcript_jsonl` enforces a strict `MAX_PAYLOAD_SIZE` ceiling (5 MB).

---

## 2. Verification Results

- **Unit Tests**: 37/37 passing in `ssh-core` (including traversal, option injection, and oversized payload tests).
- **Static Analysis**: `cargo clippy -p ssh-core -- -D warnings` clean (0 warnings).
- **Zero Battery Drain**: Transcript queries run over ephemeral SSH exec channels on the active connection; no background polling timers or sockets survive `sceneDidEnterBackground`.

```markdown
VERDICT: CLEAR
```
