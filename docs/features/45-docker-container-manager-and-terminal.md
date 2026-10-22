# Feature 45: Docker Container Manager & In-Container Terminal Sessions

- **Slug**: `docker-container-manager-and-terminal`
- **Target Release**: Milestone 34 (Remote Container & Service Management)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (executing remote docker commands, container shell attach, container name sanitization)
- **Status**: Reviewed: fix status parsing, metrics and confirmations (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Modern backend and AI development environments heavily rely on Docker containers running on the remote host (e.g. Postgres databases, Redis, Vector DBs like Qdrant/Chroma, local LLM inferencing engines via Ollama or vLLM, and agent sandboxes).
Currently, if a database container crashes or a developer needs to inspect container logs while away from their laptop:
1. They must open a raw terminal.
2. Manually type `docker ps -a` on a virtual mobile keyboard.
3. Copy a 12-character hexadecimal container ID.
4. Type `docker restart <id>` or `docker logs -f <id>`.
5. If they need to enter the container, they must type `docker exec -it <id> /bin/bash`.

This creates high mobile friction and mistakes.

### 1.2 In Scope
1. **Container Telemetry & Status Card**:
   - Integrated into the existing Host Telemetry sheet (`docs/features/09-host-telemetry-and-process-monitor.md`).
   - Displays all containers on the host with status:
     - Running: Green dot (`●`)
     - Exited / Dead: Red dot (`●`)
     - Restarting / Paused: Amber dot (`●`)
   - Resource metrics per container: memory usage (`142 MB / 4 GB`) and CPU percentage.
2. **One-Tap Actions on Each Container Card**:
   - `[ ↺ Restart ]`: Instant container restart via `docker restart <id>` with haptic feedback.
   - `[ ⎘ Logs ]`: Dedicated slide-over sheet showing the last 200 lines of logs (`docker logs --tail 200 <id>`) with Copy and Live Follow toggles.
   - `[ ▶ Start ]` / `[ ■ Stop ]`: Quick lifecycle toggles.
3. **First-Class In-Container Session on the Main Screen (SessionsHomeView)**:
   - When tapping `[ >_ Terminal ]` / `[ Exec Shell ]` on any container card:
     - A new first-class `TabSession` is created with `agentType = .dockerContainer`.
     - Automatically assigned title: `[docker] <container_name>` with icon `shippingbox.fill`.
     - Remote command:
       ```bash
       docker exec -it <sanitized_container_id_or_name> sh -c "command -v bash >/dev/null 2>&1 && exec bash || exec sh"
       ```
     - Backed by an isolated remote tmux session: `docker-<container_slug>`.
     - **Main Screen Appearance**: The session instantly appears on the app's **Main Screen (`SessionsHomeView`)** as a live card alongside Claude Code and Shell sessions, complete with real-time terminal output preview (e.g. `postgres=#` or `# /app`), active status badge, and server name.
     - **Session Persistence & Zero Battery Drain**: When the app backgrounds, the TCP connection drops cleanly while tmux keeps the container shell alive on the host. Reopening the app from the Main Screen immediately reattaches to the container session.

### 1.3 Out of Scope
- Full Docker Compose file editing or Docker build orchestration on the phone (focused on monitoring, restarts, logs, and interactive shell).

---

## 2. Domain Models & Protocol Contracts

### 2.1 Rust Core Domain Models (`crates/core/src/docker.rs`)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    Running,
    Exited,
    Paused,
    Restarting,
    Dead,
    Unknown,
}

impl ContainerState {
    /// Parsed from the `State` field of `docker ps --format '{{json .}}'` (`running`, `exited`,
    /// `paused`, `restarting`, `created`, `dead`, `removing`), never from the human `Status` text:
    /// "Up 4 hours (Paused)" and "(unhealthy)" would read as Running there.
    pub fn from_state_field(state: &str) -> Self {
        match state.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "exited" | "created" | "removing" => Self::Exited,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "dead" => Self::Dead,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerInfo {
    pub id: String,                 // 12-char or 64-char container ID
    pub name: String,               // Friendly container name (e.g. "postgres-db")
    pub image: String,              // Image tag (e.g. "postgres:16-alpine")
    pub state: ContainerState,
    pub status_text: String,        // e.g. "Up 4 hours" or "Exited (1) 10m ago"
    pub created_at: String,
    pub ports: String,              // e.g. "0.0.0.0:5432->5432/tcp"
    pub unhealthy: bool,            // from the "(unhealthy)" suffix of Status, shown as a badge only
}

/// Optional and separate: `docker stats --no-stream --format '{{json .}}'` takes 1-2 s, so the
/// list never waits for it; it is fetched on demand for the open detail view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerStats {
    pub memory_usage_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_percentage: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainersSnapshot {
    pub docker_available: bool,
    pub error_message: Option<String>,
    pub containers: Vec<DockerContainerInfo>,
}
```

### 2.2 UniFFI Interface

```rust
#[uniffi::export]
impl SshSessionHandle {
    /// Queries Docker containers on the remote host (via helper or docker CLI).
    pub async fn fetch_docker_containers(&self) -> Result<DockerContainersSnapshot, SshCoreError>;

    /// Optional resource numbers of one container (a separate, slower call).
    pub async fn fetch_docker_stats(&self, container_id: String) -> Result<DockerContainerStats, SshCoreError>;

    /// Restarts or stops a container. The app asks for confirmation first (it may be a
    /// production database); the core refuses an empty or invalid identifier.
    pub async fn restart_docker_container(&self, container_id: String) -> Result<bool, SshCoreError>;

    /// Fetches the recent log tail of a specific container.
    pub async fn fetch_docker_logs(&self, container_id: String, tail_lines: u32) -> Result<String, SshCoreError>;

    /// Generates the verified shell command to exec into the container's interactive shell.
    pub fn build_docker_exec_command(&self, container_id: String) -> Result<String, SshCoreError>;
}
```

### 2.3 Swift Types & View Models (`ios/Sources/Models/DockerContainer.swift`)

```swift
import Foundation
import SshCoreBridge

public struct DisplayableContainer: Identifiable {
    public var id: String { raw.id }
    public let raw: DockerContainerInfo
    public var stats: DockerContainerStats?   // filled lazily in the detail view

    public var isRunning: Bool { raw.state == .running }
    public var stateGlyph: String {
        switch raw.state {
        case .running: return "●"
        case .exited, .dead: return "●"
        case .paused, .restarting: return "●"
        case .unknown: return "○"
        }
    }
    public var stateColor: Color {
        switch raw.state {
        case .running: return .green
        case .exited, .dead: return .red
        case .paused, .restarting: return .orange
        case .unknown: return .secondary
        }
    }
    public var memoryDisplay: String {
        guard let usage = stats?.memoryUsageBytes, let limit = stats?.memoryLimitBytes, limit > 0 else {
            return "RAM: --"
        }
        let uMb = usage / (1024 * 1024)
        let lMb = limit / (1024 * 1024)
        return "\(uMb) MB / \(lMb) MB"
    }
}
```

---

## 3. Remote Protocols & Command Execution

### 3.1 Host Probing Protocol
To minimize round-trips and keep CPU overhead near zero, the host query runs via:
1. **Helper Path (`ssh-app-host docker-list`)**:
   - The helper has no HTTP client, so it runs the `docker` CLI itself (fixed argv, no shell, 5 s limit) and returns the JSON lines unchanged. On macOS with Docker Desktop the CLI finds its own socket (`~/.docker/run/docker.sock`); the helper never opens a socket path itself.
2. **POSIX Shell Fallback**:
   - If the helper is absent, executes a single-shot POSIX command:
   ```bash
   if command -v docker >/dev/null 2>&1; then
     docker ps -a --format '{{json .}}'
   else
     echo "DOCKER_NOT_INSTALLED"
   fi
   ```

### 3.2 In-Container Terminal Tab Spawn & Tmux Resiliency
When the user taps `[ >_ Terminal ]`:
1. The container ID is validated via `validate_docker_container_identifier(container_id)` and clamped to alphanumeric hex or sanitized slug.
2. The tmux session name is slugified to prevent tmux delimiter collisions (`.` and `:` mapped to `-`):
   ```rust
   // `a.b` and `a-b` both slugify to `a-b`, so a short id suffix keeps them apart:
   let session_name = format!("docker-{}-{}", sanitize_tmux_session_name(&container.name), &container.id[..6]);
   ```
3. The shell command is wrapped with a termination trap to prevent the tmux pane from abruptly disappearing if the container stops or exits:
   ```bash
   docker exec -it <safe_id> sh -c "command -v bash >/dev/null 2>&1 && exec bash || exec sh" || (printf '\r\n\033[33m[Container session ended (exit code %s)]\033[0m\r\nPress Enter to close.\r\n' "$?" && read -r)
   ```
4. Swift registers the new tab in `TerminalTabsManager` with `agentType = .dockerContainer`.
5. The session appears immediately on the Main Screen (`SessionsHomeView`) as a live card with a preview.
6. If the container stops or exits, the pane prints the exit code and waits for an Enter keypress before closing cleanly.

---

## 4. UI Layout & Wireframes (Strict No-Emoji Design System)

### 4.1 Main Screen (`SessionsHomeView`): Container Session Card

On the app's home dashboard, container sessions sit side-by-side with AI agent and host sessions:

```text
┌────────────────────────────────────────────────────────┐
│ [Sessions]               macbook-pro             [ ⚙ ] │
├────────────────────────────┬───────────────────────────┤
│ [brain] Claude Code    ●   │ [shippingbox.fill] postgres ●
│ macbook-pro · term-claude  │ macbook-pro · docker-pg   │
│ ┌────────────────────────┐ │ ┌───────────────────────┐ │
│ │ >_ cargo test --lib    │ │ │ psql (16.2)           │ │
│ │ 214 passed             │ │ │ Type "help" for help. │ │
│ │                        │ │ │ postgres=# \dt        │ │
│ └────────────────────────┘ │ └───────────────────────┘ │
├────────────────────────────┼───────────────────────────┤
│ [terminal] Shell       ●   │ [ + New Session ]         │
│ macbook-pro · term-1       │                           │
└────────────────────────────┴───────────────────────────┘
```

### 4.2 Host Telemetry Section: Docker & Services

```text
┌────────────────────────────────────────────────────────┐
│ [shippingbox.fill] Docker Containers (3 Active)  [ ↺ ] │
├────────────────────────────────────────────────────────┤
│ ● postgres-16                           Up 4 hours     │
│   Image: postgres:16-alpine             Port: 5432     │
│   RAM: 148 MB / 4096 MB                 CPU: 0.8%      │
│   [ ↺ Restart ]   [ ⎘ Logs ]   [ >_ Terminal ]         │
├────────────────────────────────────────────────────────┤
│ ● ollama-runner                         Up 12 hours    │
│   Image: ollama/ollama:latest           Port: 11434    │
│   RAM: 4.8 GB / 16 GB                   CPU: 12.4%     │
│   [ ↺ Restart ]   [ ⎘ Logs ]   [ >_ Terminal ]         │
├────────────────────────────────────────────────────────┤
│ ● redis-cache                           Exited (0)     │
│   Image: redis:7.2-alpine                              │
│   [ ▶ Start ]     [ ⎘ Logs ]   [ >_ Terminal ]         │
└────────────────────────────────────────────────────────┘
```

### 4.2 Container Logs Slide-Over Sheet

```text
┌────────────────────────────────────────────────────────┐
│ [ < Close ]      Logs: postgres-16               [ ⎘ ] │
│                  200 lines · Auto-scroll: ON           │
├────────────────────────────────────────────────────────┤
│ 2026-10-20 14:02:11 UTC [1] LOG: starting PostgreSQL   │
│ 2026-10-20 14:02:11 UTC [1] LOG: database system ready │
│ 2026-10-20 14:15:00 UTC [24] LOG: connection received │
│                                                        │
│                                                        │
├────────────────────────────────────────────────────────┤
│ [ doc.on.doc Copy All ]             [ ↺ Fetch Fresh ] │
└────────────────────────────────────────────────────────┘
```

---

## 5. Security Considerations (Hacker Gate)

| Threat Vector | Mitigation Strategy |
| :--- | :--- |
| **Container ID / Name Shell Injection** | Identifiers must match strict regex: `^[a-zA-Z0-9][a-zA-Z0-9_.-]{1,127}$`. Any string containing whitespace, semicolons, backticks, or pipes is rejected with `SshCoreError::InvalidIdentifier`. |
| **Privilege Escalation via Docker Exec** | `docker exec` runs with the container's default user. The app does NOT inject `--privileged` or `--user root`. |
| **Socket Hijacking / DoS** | Helper accesses `/var/run/docker.sock` strictly with timeout (max 5 s). If the user does not have permission to access the socket (not in `docker` group), a clean typed error `DockerPermissionDenied` is returned rather than hanging. |
| **Docker group is root** | Membership in the `docker` group is root on the host. The app says so in the first-use sheet and never adds the user to the group. A shell inside a container does not pass through the hook guard (the guard watches the agent's tool calls, not a terminal tab). |
| **Destructive actions** | Restart and Stop show a confirmation naming the container; there is no one-tap restart. |
| **Output Flooding on Logs** | The logs command is strictly clamped with `--tail 200` and maximum output buffer of 1 MB (excess cut with the line `[Log truncated at 1 MB limit]`), preventing iOS memory spikes. Log text and container names are scrubbed of control characters and ANSI/OSC sequences (`strip_ansi_codes`) before they are shown; the in-container terminal is SwiftTerm, which does not write the clipboard unprompted. |

---

## 6. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_validate_container_identifier`: accepts `web-app_1`, `postgres-db`, `e3b0c44298fc`; rejects `app; rm -rf /`, `app$(whoami)`.
   - `test_parse_docker_ps_json_stream`: validates parsing of single and multiple container records.
   - `test_state_field_wins_over_status_text`: `State=running` with `Status="Up 4 hours (Paused)"` is Running only when State says so; `paused` is Paused; `created` is Exited; `(unhealthy)` sets the badge, not the state.
   - `test_session_names_do_not_collide`: containers `a.b` and `a-b` get different tmux names.
   - `test_build_docker_exec_command_format`: verifies properly escaped exec invocation.
2. **Swift UI Tests**:
   - Restart/Stop ask for confirmation, then show the loading state and haptic feedback.
   - Logs viewer scrolling and clipboard copy action.
   - Spawning in-container terminal tab and verifying tab selection in `TerminalTabsManager`.

---

## Review (`rev`, 2026-10-20) - applied

All items are folded into sections 2, 3, 5 and 6 above: state from the `State` field, optional separate stats call, confirmation for Restart/Stop, short id suffix in tmux names, CLI instead of a socket client, plain statement about the `docker` group and the guard.

## Built 2026-10-22

- `crates/core/src/docker.rs`: identifier check `[A-Za-z0-9][A-Za-z0-9_.-]{1,127}` (the only variable text in any command, always single-quoted), `State` field over `Status` text, unhealthy badge, listing capped at 200, names and image scrubbed of escape sequences, `docker stats` parsed in SI and IEC units, actions (start, stop, restart) confirmed by a `DOCKER_OK` line, logs `--tail` 200 (max 1000) cut at 1 MB with the truncation line, session names `docker-<slug>-<6 of the id>` (`a.b` and `a-b` never collide), the exec command (bash or sh, exit code shown, waits for Enter, no `--privileged`, no `--user`). Everything runs through the user's login shell under a host-side watchdog; trouble (no docker, daemon down, no permission) is a message, not an error.
- `SshSessionHandle`: `fetch_docker_containers`, `fetch_docker_stats`, `docker_container_action`, `fetch_docker_logs`.
- iOS: a "Docker containers" section in the Host Monitor sheet (loaded only when opened; Restart and Stop name the container and ask first; Logs sheet with Copy and Fetch fresh; Stats on demand; Terminal opens a new tab `[docker] <name>` of the new type `dockerContainer`, icon `shippingbox.fill`, in its own tmux session). The docker-group-is-root note is shown under the list.
- Tests: 9 Rust tests (one runs the exec command for real against a container that does not exist; one lists the real docker of the machine read-only and skips without a daemon) and 1 Swift test. The developer's containers are never started, stopped or restarted by a test.
- Decision: the helper path `ssh-app-host docker-list` is **not built**. The shell command works on every host and the helper would add a command, Linux builds and a pinned hash without making anything safer or more reliable; the section 3.1 helper path stays a design option, not a plan.
- Live Follow (2026-10-22): the logs sheet has a *Live* switch that fetches the log again every 3 s while the sheet is open and the app is in front; the loop stops with the switch, the sheet or the background (no `docker logs -f` stream, so no process is left on the host).
- Shortcuts (2026-10-22): `RestartDockerContainerIntent` (feature 48) lists the containers of the chosen server, requires an **exact, single** name match (no prefix, no case folding, a name twice means refuse), asks "Restart X on Y?" with the system confirmation, and needs the device unlocked on this phone (`.requiresLocalDeviceAuthentication`). The id sent to the host is the one Docker listed, checked again by the core's identifier rule.
- Not done: a card on the home screen with a live preview (the tab appears with the other tabs). Not tried on a device.

Security (hacker gate): no free text reaches a command except the validated identifier; no privileged exec; output scrubbed before display; log size and container count bounded; a restart or stop always shows a confirmation naming the container. Nothing found.

## Own screens and "Docker unavailable" (2026-10-22)

The host monitor is split into four screens, each its own quick-menu tile (hideable, reorderable): **Host monitor** (summary, gauges), **Processes**, **Docker**, **Host helper**. The Docker screen asks the host for the list as it opens and not before. When Docker cannot be used the core says why as a value (`DockerUnavailable`: `NotInstalled`, `DaemonDown`, `NoPermission`, `Unexpected`) and the screen shows a message for each cause with the way out (install; start Docker Desktop / OrbStack / Colima or `systemctl start docker`; add the user to the `docker` group, which is root-equivalent and therefore never done for the user; run `docker ps` on the host) and a Try again button. Without a connection the screen says the tab is not connected. An SSH failure while asking shows the error under the title.
