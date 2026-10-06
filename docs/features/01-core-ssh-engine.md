# Feature Specification: Core SSH & tmux Session Engine

- **Slug**: `01-core-ssh-engine`
- **Milestone**: M1 (Architecture Spec) & M2 (Rust Core Implementation)
- **Participating Roles**: `doc-dev`, `rust-core-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (Key generation, Keychain storage, SSH auth, remote command execution)

---

## 1. Problem Statement & Scope

The iOS application needs a high-performance, asynchronous SSH client core written in pure Rust (`crates/core`) that can:
1. Generate cryptographic Ed25519 keypairs and export OpenSSH public keys.
2. Establish secure SSH sessions over Tailscale (or direct IP) to a macOS host using `russh`.
3. Detect the host login shell (`$SHELL`) dynamically and attach to or create isolated `tmux` sessions per agent tab (*Claude Code*, *Antigravity*, *Local LLM*).
4. Stream terminal I/O (ANSI/UTF-8) with dynamic PTY resizing (`window_change` / `SIGWINCH`).
5. Buffer and throttle high-bandwidth output to protect the iOS UI (SwiftTerm) from freezing during bursty output.
6. Support the "Zero Battery Drain" lifecycle: immediately close sockets when iOS suspends the app, and instantaneously reconnect and re-attach to the existing tmux session when resuming.

---

## 2. Domain Models & UniFFI Interface

### 2.1. Cryptography & Key Management

```rust
pub struct KeypairResult {
    pub public_key_openssh: String, // "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5..."
    pub private_key_openssh: String, // OpenSSH PEM/Armored format
}

pub fn generate_ed25519_keypair() -> Result<KeypairResult, SshError>;
```

### 2.2. Session Configuration & State

```rust
pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_pem: String,
    pub session_name: String,        // e.g., "claude", "antigravity", "local-llm"
    pub initial_cols: u16,
    pub initial_rows: u16,
}

pub enum SessionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Failed { reason: String },
}

pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}
```

### 2.3. Callback Trait (Swift Consumer)

```rust
#[uniffi::export(callback_interface)]
pub trait SshSessionCallback: Send + Sync {
    fn on_state_changed(&self, state: SessionState);
    fn on_data_received(&self, data: Vec<u8>);
    fn on_error(&self, message: String);
}
```

### 2.4. Session Controller Handle

```rust
#[uniffi::export]
impl SshSessionHandle {
    pub fn connect(&self, callback: Box<dyn SshSessionCallback>) -> Result<(), SshError>;
    pub fn send_input(&self, data: Vec<u8>) -> Result<(), SshError>;
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), SshError>;
    pub fn disconnect(&self); // Zero Battery Drain: gracefully closes socket
    pub fn reconnect(&self) -> Result<(), SshError>; // Instant re-attach
}
```

---

## 3. Remote Shell & tmux Orchestration

### 3.1. Dynamic Environment Initialization
When an SSH channel opens:
1. Query or inherit remote login environment:
   The core invokes the user's default login shell without hardcoding paths:
   ```bash
   exec $SHELL -l -c "<tmux_command>"
   ```
2. Dynamic fallback: if `$SHELL` is unset in the remote SSH environment, fallback to `/bin/zsh -l -c` or `/bin/bash -l -c`.

### 3.2. tmux Session Command Structure
Each tab corresponds to an isolated tmux session:
```bash
tmux new-session -A -s <safe_session_name>
```
- `-A`: Attach to existing session if it exists, otherwise create it.
- `-s <safe_session_name>`: Session identifier.
- Parameter sanitization: `safe_session_name` MUST match regex `^[a-zA-Z0-9_\-\.]+$` to strictly prevent command injection.

### 3.3. TrueColor Compatibility
Ensure tmux is started or configured with 24-bit TrueColor support:
```bash
set -ga terminal-overrides ",xterm-256color:Tc"
```
The PTY is requested with TERM `xterm-256color`.

---

## 4. Throttling Buffer for SwiftTerm UI Protection

To prevent high-volume terminal output (e.g., `cat 100MB.log`) from overwhelming the UniFFI bridge and iOS main thread:
1. Incoming bytes from `russh::Channel::data` are queued into an internal buffer.
2. A flush timer or batching window (e.g. 16ms / ~60 FPS or when buffer reaches 16KB) coalesces chunks into a single `on_data_received(batch)` callback invocation.
3. This eliminates thousands of micro-allocations across the C-FFI boundary and guarantees smooth 60/120Hz SwiftTerm rendering.

---

## 5. Zero Battery Drain Lifecycle

1. **Active State**:
   - Connection established over Tailscale IP.
   - Pings sent every 30–45s if idle (`russh` keepalive).
2. **Background State (`sceneDidEnterBackground`)**:
   - Swift calls `session.disconnect()`.
   - Rust closes the SSH channel and TCP connection cleanly.
   - 0 background tasks, 0 socket listeners, 0 CPU cycles on iOS.
   - On the MacBook, `tmux` continues executing agent workloads detached.
3. **Foreground State (`didBecomeActive`)**:
   - Swift calls `session.reconnect()`.
   - Rust opens a new TCP connection, performs Ed25519 authentication, issues `tmux attach-session -t <safe_session_name> -d` (detaching any stale client), and restores streaming.

---

## 6. Security Considerations (Hacker Gate)

1. **Host Key Verification**: Support known host key checking (TOFU — Trust On First Use or pinned host key) to prevent MITM attacks.
2. **CSPRNG & Zeroization**: Ed25519 keypair generation using `ssh_key::PrivateKey::random(&mut rand::rngs::OsRng)`. Sensitive key buffers implement `zeroize::ZeroizeOnDrop`.
3. **Command Sanitization**: Session names and environment variables strictly validated against a whitelist of characters. No arbitrary shell string interpolation.

---

## 7. Verification & Acceptance Criteria

- [ ] Cargo workspace compiles cleanly with Rust 2021.
- [ ] Ed25519 key generation passes unit tests, validates OpenSSH format roundtrip.
- [ ] Session name validator rejects dangerous shell metacharacters (`;`, `&`, `|`, `` ` ``, `$()`, `\n`).
- [ ] Throttling buffer coalesces rapid bursts and flushes promptly on idle.
- [ ] Code passes `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
