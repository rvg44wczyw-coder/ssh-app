# Security findings: feature 33 H3/H4, WSL2 snippets and host-aware helper pin (2026-10-19)

## 1. Scope

`crates/core/src/wsl.rs` (`wsl_setup_snippets`, `validate_public_key`), `hosttarget.rs`
(`host_target_label`, `host_target_hides_battery`), Swift: `HostTargetCache`, `helperPin()` in
`TerminalSessionViewModel`, `HostHelperCard` / `WslSetupView`.

Applies: text the user pastes into a privileged shell (snippet injection), choice of the pinned
digest from a host-reported system. Ruled out: crypto, replay, timing, network input (nothing is sent).

Live tests: unit tests feeding hostile keys (quote and `$(…)` in the comment/blob, embedded newline,
extra fields, wrong type, short blob) and ports (0, non-numeric by type); the snippets were read for
every route. Code analysis: the Swift cache (lock-protected map), the fall-back pin.

## 2. Findings

### 1. [Informational] Snippets are pasted as administrator
The PowerShell snippets run as administrator and contain only fixed text and the port number (a
`u16`); the shell snippets contain the key line, which is restricted to `ssh-ed25519`, base64
alphabet and a comment of `[A-Za-z0-9@._-]` (no quote, space, `$`, backtick or newline), so it cannot
break out of its single quotes. The `tailscale.com/install.sh` line is the vendor's documented install
command and is shown for the user to run knowingly.

### 2. [Informational] A lying host cannot lower the pin
`helperPin()` takes the digest of the app's own blob for the reported system (as in H1); a wrong claim
gives a blob that does not run, and all calls fall back as before. Before detection, or if it fails,
the Mac build's digest is used (the old behaviour).

### 3. [Low, fixed] Telemetry battery of a WSL VM
Not a security issue: battery data of a VM was shown as the PC's; the card now hides it for WSL.

## 3. Verdict

Findings (highest severity: Low, resolved). Clean for H3/H4.
