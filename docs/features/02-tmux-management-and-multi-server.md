# Feature Specification: Remote tmux Management & Multi-Server Support

- **Slug**: `02-tmux-management-and-multi-server`
- **Milestone**: M3 (Multi-Host & Session Lifecycle)
- **Participating Roles**: `doc-dev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `ios-ui-dev`, `hacker`
- **Hacker Gate**: Yes (SSH authentication to arbitrary hosts, remote command execution, tmux process termination)

---

## 1. Problem Statement & Scope

### 1.1. Problem Statement
1. **Tmux Session Visibility & Lifecycle Control**: Currently, the iOS app attaches to or creates a tmux session (`term-1`, `term-2`, etc.) by name, but users cannot inspect which tmux sessions already exist on the remote host, which ones are still running background agent jobs (Claude Code, Antigravity, long builds), or terminate/cleanup orphaned sessions without manually typing tmux CLI commands.
2. **Multiple SSH Server Profiles**: Currently, server configuration is stored as a single global set of `@AppStorage` keys (`ssh_host`, `ssh_port`, `ssh_username`). Users need to seamlessly switch between multiple remote machines (e.g. MacBook Pro over Tailscale, a Linux home lab server, or cloud VPS instances), maintaining separate tmux sessions and terminal tabs across different hosts.

### 1.2. Scope
- **In Scope**:
  - Rust Core: One-shot SSH command execution helper (`execute_remote_command`) over `russh` with strict timeout.
  - Rust Core: Typed `TmuxSessionInfo` record and parser for `tmux list-sessions` output.
  - Rust Core: Functions to list active tmux sessions (`list_remote_tmux_sessions`) and kill/stop a session (`kill_remote_tmux_session`) with strict session name validation to prevent command injection.
  - UniFFI Export & Swift Bridge generation.
  - iOS UI: `ServerProfile` model with UUID, name/label, host, port, username, and default flag, persisted via `UserDefaults`.
  - iOS UI: Multi-server CRUD in Settings (Add, Edit, Delete, Select Active Server).
  - iOS UI: Dedicated "Tmux Sessions" management sheet/panel accessible from the toolbar:
    - Host selector (if multiple hosts configured).
    - Real-time list of active tmux sessions with window count, status (attached/detached), and creation date.
    - "Open in New Tab" / "Attach" action.
    - "Stop / Remove" (`kill-session`) action with confirmation dialog.
    - Manual refresh button with loading indicator.
  - iOS UI: Tab sessions bound to specific `ServerProfile` instances so different tabs can connect to different servers simultaneously.
- **Out of Scope**:
  - Remote tmux pane splitting (split horizontal/vertical) inside a single terminal view (future feature).
  - SSH tunneling / port forwarding (future feature).

---

## 2. Domain Models & UniFFI Interface

### 2.1. Rust Domain Models (`crates/core`)

```rust
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: bool,
    pub created_timestamp: u64,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct RemoteServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_pem: String,
}
```

### 2.2. UniFFI Exported Functions

```rust
#[uniffi::export]
pub async fn list_remote_tmux_sessions(
    config: RemoteServerConfig,
) -> Result<Vec<TmuxSessionInfo>, SshCoreError>;

#[uniffi::export]
pub async fn kill_remote_tmux_session(
    config: RemoteServerConfig,
    session_name: String,
) -> Result<(), SshCoreError>;

#[uniffi::export]
pub async fn execute_remote_command(
    config: RemoteServerConfig,
    command: String,
) -> Result<String, SshCoreError>;
```

### 2.3. Swift Models (`ios/Sources/SSHApp/Models`)

```swift
public struct ServerProfile: Identifiable, Codable, Equatable, Hashable {
    public let id: UUID
    public var name: String
    public var host: String
    public var port: UInt16
    public var username: String
    public var isDefault: Bool
}
```

---

## 3. Remote Shell & tmux Protocols

### 3.1. Querying Active tmux Sessions
To list sessions in a structured machine-readable format without parsing brittle human strings:
```bash
tmux list-sessions -F "#{session_name}|#{session_windows}|#{?session_attached,1,0}|#{session_created}" 2>/dev/null || true
```
- Format fields:
  - `#{session_name}`: alphanumeric + hyphen/underscore string.
  - `#{session_windows}`: integer count of windows.
  - `#{?session_attached,1,0}`: 1 if a client is currently attached, 0 otherwise.
  - `#{session_created}`: Unix epoch timestamp in seconds.
- If tmux is not running or has no sessions, the command returns exit code 0 with empty stdout.

### 3.2. Stopping / Removing a tmux Session
```bash
tmux kill-session -t '<session_name>'
```
The session name is validated with `validate_session_name` in Rust before sending to prevent shell injection.

---

## 4. State Machines & Lifecycle

### 4.1. Server Profiles Persistence & Migration
- Existing settings (`ssh_host`, `ssh_port`, `ssh_username` in `UserDefaults`) are automatically migrated into a default `ServerProfile` ("Primary Host") on first launch if no profiles exist.
- Profiles are stored under `UserDefaults` key `"saved_server_profiles"`.
- A single active server profile ID is stored under `"active_server_profile_id"`.

### 4.2. Tab Binding to Servers
- Each `TabSession` stores an optional `serverId: UUID?`.
- If `serverId` is nil or matches a deleted profile, it falls back to the default/active server.
- Switching tabs switches the terminal view to that tab's server and session.
- Backgrounding tears down sockets for all tabs (Zero Battery Drain). Resuming reconnects active tabs to their respective hosts and tmux sessions.

### 4.3. Tmux Manager Lifecycle
- Presented as a sheet from the toolbar via a session icon button.
- Fetches sessions asynchronously on appear.
- Allows killing a session (with confirmation alert).
- Allows tapping a session to attach/open it as an iOS tab.

---

## 5. Security Considerations

1. **Session Name Validation**:
   - `validate_session_name` enforces regex `^[a-zA-Z0-9_-]{1,64}$`.
   - Rejects any characters like `;`, `&`, `|`, spaces, quotes, newlines.
2. **Command Parameterization**:
   - Single quotes inside SSH commands are safely escaped.
3. **Keychain Private Key**:
   - Uses the shared Ed25519 private key stored with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
   - Zeroized on drop in Rust.
4. **Error Masking**:
   - Remote error messages returned to Swift do not leak sensitive file paths or private key headers.

---

## 6. Verification & Testing Criteria

1. **Rust Core Tests**:
   - Parsing test for `parse_tmux_sessions_output` (empty output, single session, multiple sessions, malformed rows).
   - Session name validation tests for `kill_remote_tmux_session`.
   - Command builder verification.
2. **Swift & iOS Tests**:
   - `ServerProfile` encoding/decoding and migration tests.
   - UI navigation & session manager sheet rendering without freezing.
   - Build succeeds on target device (`aarch64-apple-ios`).
