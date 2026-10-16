# Feature 09: Remote Host Telemetry & Agent Process Monitor

- **Slug**: `09-host-telemetry-and-process-monitor`
- **Milestone**: M12
- **Participating Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate Required**: `Yes` (executes remote process management and telemetry collection over SSH)

---

## 1. Problem Statement & Scope

When running long-lived AI agents (Claude Code, Google Antigravity, Ollama local models) and resource-intensive compilation jobs (`cargo`, `xcodebuild`, `node`, `docker`) on a remote MacBook over Tailscale, mobile users lack immediate visibility into host health:
- Is the remote MacBook running on battery power and about to sleep or shut down?
- Is an Ollama model or stuck `cargo` build exhausting RAM or pinning all CPU cores?
- Is disk space running critically low during builds?
- Can a runaway agent or hung build process be cleanly terminated (`SIGTERM` / `SIGKILL`) with a single tap without opening a new terminal tab and running `ps aux | grep ...` manually?

### In Scope
- **Rust Core (`crates/core/src/telemetry.rs` & `session.rs`)**:
  - Single-shot, zero-interpolation remote telemetry collection script supporting both **macOS** (`sysctl`, `vm_stat`, `pmset -g batt`, `df -k /`, `ps`) and **Linux** (`/proc/loadavg`, `/proc/meminfo`, `df -k /`, `ps`).
  - Deterministic parser `parse_host_telemetry(raw_output: &str) -> HostTelemetrySnapshot` extracting:
    - Hostname, OS name, uptime summary.
    - CPU load averages (`load_1m`, `load_5m`, `load_15m`) and logical core count.
    - Memory usage (`memory_used_mb`, `memory_total_mb`, `memory_usage_percent`).
    - Disk usage (`disk_used_gb`, `disk_total_gb`, `disk_usage_percent`).
    - MacBook battery status (`battery_percent: Option<u8>`, `is_charging: Option<bool>`, `power_source: Option<String>`).
    - Top developer & AI agent processes (`Vec<RemoteProcessInfo>`) classified into `ProcessCategory` (`AiAgent`, `LocalLlm`, `BuildTool`, `Runtime`, `System`).
  - Safe remote process termination `kill_remote_process(pid: u32, force: bool)` on `SshSessionHandle` enforcing `pid > 1`.
- **UniFFI / Swift Bridge**:
  - Export `ProcessCategory`, `RemoteProcessInfo`, `HostTelemetrySnapshot`, `parse_host_telemetry`, `SshSessionHandle::fetch_host_telemetry`, and `SshSessionHandle::kill_remote_process`.
- **iOS UI (`ios/Sources/SSHApp/Views/HostTelemetrySheetView.swift`)**:
  - Top bar gauge button (`gauge.with.dots.needle.33percent` / `chart.bar.xaxis`) opening `HostTelemetrySheetView`.
  - Real-time gauges for CPU Load, RAM, Disk, and MacBook Battery/AC status.
  - Interactive list of AI Agent & Developer processes with one-tap `Terminate (SIGTERM)` and confirmation-protected `Force Kill (SIGKILL)`.

### Out of Scope
- Continuous background daemon polling on the host when the sheet is closed.
- Modifying system power settings (`pmset` write operations).

---

## 2. Domain Models & Protocol Contracts

### 2.1 Rust Core Types (`crates/core/src/telemetry.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum ProcessCategory {
    AiAgent,
    LocalLlm,
    BuildTool,
    Runtime,
    System,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct RemoteProcessInfo {
    pub pid: u32,
    pub name: String,
    pub command_line: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub elapsed: String,
    pub category: ProcessCategory,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct HostTelemetrySnapshot {
    pub hostname: String,
    pub os_name: String,
    pub cpu_cores: u32,
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub memory_usage_percent: f64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub disk_usage_percent: f64,
    pub battery_percent: Option<u8>,
    pub is_charging: Option<bool>,
    pub power_source: Option<String>,
    pub processes: Vec<RemoteProcessInfo>,
}
```

### 2.2 UniFFI Functions & Session Methods

```rust
#[uniffi::export]
pub fn parse_host_telemetry(raw_output: String) -> HostTelemetrySnapshot;

#[uniffi::export]
pub fn classify_process_category(comm: String) -> ProcessCategory;

// On SshSessionHandle:
pub async fn fetch_host_telemetry(&self) -> Result<HostTelemetrySnapshot, SshCoreError>;
pub async fn kill_remote_process(&self, pid: u32, force: bool) -> Result<(), SshCoreError>;
```

---

## 3. Remote Environment Protocol

`fetch_host_telemetry` executes a single static shell command over an ephemeral SSH `exec` channel using clearly delimited sections (`===HOST===`, `===LOAD===`, `===MEM===`, `===DISK===`, `===BATT===`, `===PROC===`) so that a single round-trip (< 100ms over Tailscale) gathers all metrics without spawning multiple SSH channels.

---

## 4. State Machine & Zero Battery Drain Lifecycle

- Telemetry is fetched **only** when `HostTelemetrySheetView` is presented (`onAppear` or manual refresh button).
- When the app enters `.background` (`sceneDidEnterBackground`), the sheet does not poll and the underlying SSH session is immediately torn down per the Zero Battery Drain architecture.

---

## 5. Security Considerations

1. **Zero Command Injection**:
   - `fetch_host_telemetry` uses a compile-time constant command string (`const TELEMETRY_SCRIPT: &str`) with zero user-supplied parameters.
   - `kill_remote_process(pid: u32, force: bool)` accepts a strongly typed `u32` PID and a `bool` flag, rejecting `pid <= 1` with `SshCoreError::InvalidConfig("Refusing to signal reserved system PID")` so `kill -9 0` or `kill -9 1` (`launchd`/`init`) is impossible.
2. **Output Sanitization**:
   - All parsed strings strip ANSI escape codes (`strip_ansi_codes`) and truncate abnormally long process command lines (max 240 chars) before crossing FFI.

---

## 6. Verification & Testing Criteria

- Unit tests in `crates/core/src/telemetry.rs` covering:
  - Full macOS telemetry snapshot parsing (`vm_stat`, `sysctl`, `pmset -g batt` on AC Power and Battery Power, `df -k /`, `ps`).
  - Full Linux telemetry snapshot parsing (`/proc/meminfo`, `/proc/loadavg`).
  - Process category classification (`claude`, `antigravity`, `ollama`, `cargo`, `rustc`, `node`, `tmux`).
  - Reserved PID rejection (`pid == 0`, `pid == 1`).
- Swift package tests (`swift test --package-path packages/SshCoreBridge`) and `xcodebuild` verification.
