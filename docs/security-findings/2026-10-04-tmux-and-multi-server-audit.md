# Security Audit: Tmux Session Management & Multi-Server Support

**Date:** 2026-10-04  
**Auditor:** Hacker (Adversarial Security Auditor)  
**Target:** Tmux session manager, remote command execution, and multi-server configuration in `ssh-core` + iOS UI.

---

## 1. Scope & Vectors

1. **Remote Command Injection via tmux parameters**:
   - Analyzed `build_tmux_kill_session_command` and `build_tmux_attach_or_create_command`.
   - Validated that `validate_session_name` enforces strict regex `^[a-zA-Z0-9_\-\.]+$` with max length 64.
   - Session names are additionally escaped with `shlex::try_quote`. Shell operators (`;`, `&&`, `|`, `$(...)`, newline, quotes) are rejected before execution.

2. **Credential Storage & Multi-Server Isolation**:
   - Server profiles (`ServerProfile`) store only non-sensitive metadata (UUID, label, hostname/IP, port, username) in `UserDefaults`.
   - Private keys remain exclusively inside the iOS Keychain (`kSecClassGenericPassword`, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, `kSecAttrSynchronizable = false`).
   - Ephemeral SSH client sessions for `list_remote_tmux_sessions` and `kill_remote_tmux_session` close immediately upon command completion or 10-second timeout.

3. **Denial of Service & Connection Leaks**:
   - Remote command execution is bounded by Tokio `tokio::time::timeout(Duration::from_secs(10))` preventing hanging tasks or thread starvation.
   - Channel EOF and disconnect are explicitly awaited.

4. **Zero Battery Drain Lifecycle**:
   - Ephemeral commands don't maintain open background sockets.
   - Interactive terminal sessions preserve strict socket teardown on `sceneDidEnterBackground`.

---

## Security Audit Verdict

VERDICT: CLEAR
