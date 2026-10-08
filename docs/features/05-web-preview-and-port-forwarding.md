# Feature 05: In-App Web Preview & SSH Direct-TCPIP Port Forwarding

- **Feature Slug**: `05-web-preview-and-port-forwarding`
- **Milestone**: Milestone M8
- **Participating Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `android-ui-dev`, `rev`, `hacker`
- **Hacker Gate Required**: **Yes** (Network socket binding, SSH `direct-tcpip` tunneling, in-app `WebView` security, loopback isolation)

---

## 1. Problem Statement & Objectives

### 1.1 The User Problem
When autonomous AI agents (Claude Code, Google Antigravity, OpenAI Codex) or human developers build web services, dashboards, or full-stack applications on their remote MacBook, they typically start local HTTP dev servers:
- Next.js / React: `localhost:3000`
- Vite / Vue / Svelte: `localhost:5173`
- Python FastAPI / Flask / Uvicorn: `localhost:8000` or `localhost:5000`
- Go / Rust HTTP servers / Microservices: `localhost:8080`

When managing sessions remotely from an iPhone or Android device, the developer cannot easily view, click, or test the rendered web frontend. Opening a separate SSH client or configuring reverse proxies is slow, complex, and drains battery.

### 1.2 Objectives
1. **Direct TCP/IP Port Forwarding in Rust Core**:
   Implement SSH port forwarding natively within `crates/core` using `russh` (`direct-tcpip` channel protocol).
2. **Local Loopback Proxy**:
   Rust core binds to an ephemeral or specified local port on `127.0.0.1` on the mobile device and multiplexes incoming HTTP/TCP connections over the existing SSH connection.
3. **In-App Mobile Web Preview**:
   Provide a polished mobile browser sheet (`WKWebView` on iOS, `WebView` on Android) with:
   - Quick port selector chips (`3000`, `5173`, `8080`, `8000`, `+ Custom`).
   - Navigation controls (Back, Forward, Reload, Share, Open in Safari/Chrome).
   - In-app DevTools console drawer capturing `console.log` and `console.error`.
4. **Zero Battery Drain & Security Invariants**:
   - Zero background listener leak: all local proxy sockets and forwarded channels are immediately dropped when the mobile app backgrounds (`sceneDidEnterBackground` / `ON_STOP`).
   - Strict loopback binding: sockets bind strictly to `127.0.0.1`, preventing any device on the local Wi-Fi from reaching forwarded remote services.

---

## 2. Architecture & Domain Models

### 2.1 System Architecture

```text
┌────────────────────────────────────────────────────────────────────────┐
│                         MOBILE DEVICE (iOS / Android)                  │
│                                                                        │
│  ┌───────────────────────┐                                             │
│  │ In-App Web Preview    │ ─── HTTP GET http://127.0.0.1:<local_port>  │
│  │ (WKWebView / WebView) │                                             │
│  └───────────┬───────────┘                                             │
│              │                                                         │
│              ▼                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                      RUST CORE (crates/core)                     │  │
│  │                                                                  │  │
│  │  ┌─────────────────────────┐     ┌────────────────────────────┐  │  │
│  │  │ Local Loopback Listener │ ──► │ PortForwardManager         │  │  │
│  │  │ (127.0.0.1:<local_port>)│     │ (russh direct-tcpip pump)  │  │  │
│  │  └─────────────────────────┘     └─────────────┬──────────────┘  │  │
│  └────────────────────────────────────────────────┼─────────────────┘  │
└───────────────────────────────────────────────────┼────────────────────┘
                                                    │ SSH Tunnel
                                                    │ (ChannelMsg::OpenDirectTcpip)
                                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                       REMOTE HOST (MacBook over Tailscale)             │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ sshd Server                                                      │  │
│  │ Accepts direct-tcpip channel ──► connects to 127.0.0.1:<rem_port>│  │
│  └──────────────────────────────────┬───────────────────────────────┘  │
│                                     │ TCP stream                       │
│                                     ▼                                  │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ Remote Web Server (Next.js, Vite, FastAPI, etc. on <rem_port>)   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Rust Domain Models (`crates/core/src/port_forward.rs`)

```rust
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;
use uniffi;

/// Status of an active port forwarding tunnel.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PortForwardStatus {
    Starting,
    Listening,
    ActiveConnections { count: u32 },
    Stopped,
    Failed { reason: String },
}

/// Information about a running or requested port forward.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PortForwardInfo {
    pub remote_port: u16,
    pub local_port: u16,
    pub remote_host: String,
    pub active_connections: u32,
    pub is_running: bool,
}

/// Handle returned to Swift/Kotlin to control the lifecycle of a forwarded port.
#[derive(uniffi::Object)]
pub struct PortForwardHandle {
    pub remote_port: u16,
    pub local_port: u16,
    pub remote_host: String,
    shutdown_tx: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    active_connections: Arc<AtomicUsize>,
    is_running: Arc<AtomicBool>,
}

#[uniffi::export]
impl PortForwardHandle {
    /// Returns the local port bound on 127.0.0.1.
    pub fn get_local_port(&self) -> u16 {
        self.local_port
    }

    /// Returns the remote target port on the host.
    pub fn get_remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Returns the full local HTTP URL for the mobile WebView to load.
    pub fn get_local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.local_port)
    }

    /// Returns current number of open proxied connections.
    pub fn get_active_connections(&self) -> u32 {
        self.active_connections.load(Ordering::SeqCst) as u32
    }

    /// Stops the local listener and terminates all active TCP forward streams.
    pub fn stop(&self) {
        if self.is_running.swap(false, Ordering::SeqCst) {
            let mut lock = self.shutdown_tx.lock();
            if let Some(tx) = lock.take() {
                let _ = tx.send(());
            }
        }
    }

    /// Checks if the tunnel is currently running.
    pub fn is_active(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}
```

### 2.3 UniFFI Export Interface on `SshSessionHandle`

```rust
#[uniffi::export]
impl SshSessionHandle {
    /// Starts local port forwarding for the specified remote port.
    /// If `local_port` is 0, the OS allocates an available ephemeral loopback port.
    pub async fn start_port_forward(
        &self,
        remote_port: u16,
        local_port: Option<u16>,
    ) -> Result<Arc<PortForwardHandle>, SshCoreError>;

    /// Lists all currently active port forward handles on this session.
    pub fn get_active_port_forwards(&self) -> Vec<PortForwardInfo>;

    /// Stops all running port forwards (used during teardown/backgrounding).
    pub fn stop_all_port_forwards(&self);
}
```

### 2.4 Error Handling (`SshCoreError` extensions)

```rust
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SshCoreError {
    // ... existing variants ...

    #[error("Port forward failed: {0}")]
    PortForwardFailed(String),

    #[error("Local port {0} is already bound")]
    LocalPortInUse(u16),

    #[error("Remote host refused direct-tcpip channel for port {0}")]
    RemotePortRefused(u16),

    #[error("Direct TCP/IP channel disconnected")]
    ChannelClosed,
}
```

---

## 3. SSH Direct-TCPIP Protocol & Streaming

### 3.1 Establishing the SSH Channel
In the SSH RFC 4254 standard (§7.2), client-to-server port forwarding uses `direct-tcpip`:
- `host_to_connect`: Target hostname on remote side (usually `"127.0.0.1"` or `"localhost"`).
- `port_to_connect`: Target port on remote side (e.g. `3000`).
- `originator_ip`: Client loopback IP (`"127.0.0.1"`).
- `originator_port`: Ephemeral client port.

Using `russh`:
```rust
let channel = session.channel_open_direct_tcpip(
    "127.0.0.1",
    remote_port as u32,
    "127.0.0.1",
    client_local_port as u32,
).await.map_err(|e| SshCoreError::PortForwardFailed(e.to_string()))?;
```

### 3.2 Bidirectional Proxy Pump
When a local client (e.g. `WKWebView`) connects to `127.0.0.1:<local_port>`:
1. Accept `tokio::net::TcpStream`.
2. Open SSH `direct-tcpip` channel.
3. Split both streams into read/write halves:
   - `tokio::io::copy(&mut tcp_read, &mut ssh_write)`
   - `tokio::io::copy(&mut ssh_read, &mut tcp_write)`
4. Track connection count using `AtomicUsize`.
5. Max concurrent connection threshold: Limit to 32 concurrent streams to protect mobile memory.

---

## 4. Mobile Client UI & Developer Experience

### 4.1 In-App Web Preview Sheet
Both iOS (`WKWebView`) and Android (`WebView`) implement a cohesive sheet:
1. **Top Control Bar**:
   - Status badge (e.g. `● 127.0.0.1:3000 ⇄ remote:3000`).
   - Quick port buttons: `[3000]`, `[5173]`, `[8000]`, `[8080]`, `[ + ]`.
   - Reload button (`↻`).
   - DevTools console toggle (`[ ⌨ Console (N) ]`).
   - External browser export (`Share` / `Open in Safari/Chrome`).
2. **Main Viewport**:
   - Full-bleed HTML5 rendering with touch interaction, viewport scaling, and cookies enabled.
3. **Console Drawer (Mobile DevTools)**:
   - Injects a small non-intrusive JavaScript hook:
     ```javascript
     (function() {
       const _log = console.log;
       const _err = console.error;
       console.log = function(...args) {
         window.webkit?.messageHandlers?.consoleLog?.postMessage({level: 'log', msg: args.join(' ')});
         _log.apply(console, args);
       };
       console.error = function(...args) {
         window.webkit?.messageHandlers?.consoleLog?.postMessage({level: 'error', msg: args.join(' ')});
         _err.apply(console, args);
       };
     })();
     ```
   - Renders a collapsible bottom sheet showing timestamped logs with copy button.

### 4.2 Terminal Bar Integration
When connected, the top bar of `MainTerminalScreen` displays a `[ 🌐 Web ]` button. Tapping it opens the Web Preview sheet without interrupting the terminal or tmux session.

---

## 5. Zero Battery Drain Lifecycle & Security

### 5.1 Zero Battery Drain Invariant
- **On Background** (`sceneDidEnterBackground` on iOS, `ON_STOP` on Android):
  - `sessionHandle.stop_all_port_forwards()` is immediately invoked.
  - All local TCP listener sockets are closed.
  - Active channels terminate gracefully.
  - Radio module goes to idle (0% CPU, 0% radio usage).
- **On Foreground** (`didBecomeActive` / `ON_RESUME`):
  - If the Web Preview sheet was actively open, the last forwarded port is automatically re-established once the SSH session re-attaches.

### 5.2 Hacker Security Gate
- **Binding Restriction**: Listeners bind exclusively to `127.0.0.1`. Binding to `0.0.0.0` or `inaddr_any` is strictly prohibited.
- **SSRF Prevention**: The target host in `channel_open_direct_tcpip` is restricted strictly to `127.0.0.1` and `localhost`. Forwarding to arbitrary third-party LAN IPs through the Mac is forbidden by default.
- **WebView Sandbox**:
  - `allowFileAccess = false`
  - `allowContentAccess = false`
  - JavaScript execution allowed only for the loopback origin (`http://127.0.0.1:*`).
  - Native iOS ATS (App Transport Security) allows local loopback HTTP (`NSAllowsLocalNetworking = true`).

---

## 6. Verification & Test Plan

1. **Rust Core Unit Tests** (`crates/core/tests/` & `crates/core/src/port_forward.rs`):
   - `test_local_listener_binds_ephemeral_port`: checks local port allocation.
   - `test_port_forward_handle_stop_closes_listener`: verifies clean shutdown.
   - `test_port_forward_info_records`: verifies status and metadata.
   - `test_disallow_non_loopback_binding`: verifies rejection of non-loopback addresses.
2. **Swift Package Tests** (`packages/SshCoreBridge/Tests/`):
   - UniFFI interface compilation, handle lifecycle, error lifting.
3. **Android Integration**:
   - Kotlin coroutines integration with `PortForwardHandle`.
4. **Hacker Audit**:
   - Verification of loopback enforcement and memory zeroization.
