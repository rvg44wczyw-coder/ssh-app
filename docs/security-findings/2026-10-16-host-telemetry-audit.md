# Security Audit Report: Feature 09 — Remote Host Telemetry & Agent Process Monitor

- **Date**: 2026-10-16
- **Feature**: `docs/features/09-host-telemetry-and-process-monitor.md`
- **Auditor**: `hacker` skill gate

## 1. Threat Model & Attack Surface

1. **Shell Command Injection in Process Termination**:
   - The client transmits a PID (`u32`) and a force flag (`bool`) to terminate a remote process over SSH `exec`.
   - **Mitigation**: `pid` is strictly typed as an unsigned 32-bit integer (`u32`) across UniFFI and formatted into `kill -TERM <pid>` or `kill -KILL <pid>`. No user-supplied strings are interpolated into the shell command.

2. **Critical System Process Denial-of-Service (PID <= 1)**:
   - Sending `kill -TERM 0`, `kill -TERM 1`, or negative PIDs could signal process groups or `launchd`/`init`.
   - **Mitigation**: `validate_kill_pid(pid)` in `crates/core/src/telemetry.rs` strictly rejects `pid <= 1` (`0` and `1`) before any SSH channel is opened, covered by unit tests.

3. **Malicious Process Name / Command Line Output Spoofing**:
   - A local process on the remote host could have a crafted argv containing markers (`===TELEMETRY_SECTION:`) or newlines.
   - **Mitigation**: `parse_host_telemetry` parses `ps` output only after the last `===TELEMETRY_SECTION:PROCS===` header, uses bounded `split_whitespace` fields (`pid`, `pcpu`, `rss`, `comm`, `args`), caps the process list at 15 entries, and renders all strings in SwiftUI `Text` views (no raw terminal ANSI evaluation).

4. **Zero Battery Drain Compliance**:
   - Telemetry collection is strictly on-demand via transient SSH `exec` channels (`fetch_host_telemetry`) when the user opens or refreshes `HostTelemetrySheetView`. No background polling loops or persistent timers exist when `scenePhase == .background`.

## 2. Findings

- **Critical / High / Medium**: None (0).
- **Low / Informational**: None (0).

## 3. Verdict

**VERDICT: CLEAR** — Safe to merge.
