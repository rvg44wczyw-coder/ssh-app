# Security Audit: In-App Web Preview & SSH Direct-TCPIP Port Forwarding (Milestone M8 / Feature 05)

**Date:** 2026-10-08  
**Auditor:** Hacker (Adversarial Security Auditor)  
**Target:** Direct-TCPIP proxy engine in Rust Core (`crates/core/src/port_forward.rs`, `session.rs`), iOS `WKWebView` preview sheet (`WebPreviewSheetView.swift`), Android Compose `WebView` sheet (`WebPreviewDialog.kt`), and background lifecycle teardown.

---

## 1. Executive Summary

An adversarial security audit was conducted on the In-App Web Preview & SSH Port Forwarding feature set (Feature 05 / Milestone M8).
The objective was to evaluate the local TCP proxy, SSH direct-tcpip channel handling, mobile WebView sandboxing, and background lifecycle against attack vectors including:
1. Unauthorized local network exposure / open listening sockets on external interfaces.
2. Cross-Site Scripting (XSS) and local file inclusion via mobile WebViews.
3. Denial of Service (DoS) / socket exhaustion via unbounded concurrent connections.
4. Background socket leakage violating the Zero Battery Drain invariant.
5. WebKit script message handler memory leaks and retain cycles.

### Audit Result
- **Findings Identified & Remediated**:
  - **[REMEDIATED] VULN-01 (Low): WKUserContentController Strong Reference Leak**: `WKUserContentController.add(_:name:)` strongly retained the coordinator without cleanup in `WebPreviewSheetView.swift`. Remediated by implementing `UIViewRepresentable.dismantleUIView` with explicit `removeScriptMessageHandler(forName: "consoleLog")` and `uiView.stopLoading()`.
  - **[VERIFIED SECURE] Loopback Restriction**: Loopback server strictly binds to `127.0.0.1` and never `0.0.0.0`.
  - **[VERIFIED SECURE] Android WebView File Sandbox**: `allowFileAccess = false` and `allowContentAccess = false` explicitly enforced.
  - **[VERIFIED SECURE] Zero Battery Drain**: Background transitions on both iOS (`scenePhase == .background`) and Android (`Lifecycle.Event.ON_STOP`) immediately close all local TCP sockets and SSH channels.

**Final Verdict:** `VERDICT: CLEAR`

---

## 2. Threat Vector Analysis & Audit Details

### Vector 1: Local Listener Binding & Network Isolation
- **Threat**: Binding a local proxy listener to wildcard addresses (`0.0.0.0` or `::`) would expose the forwarded service to any device on the same local Wi-Fi or cellular network, allowing unauthorized remote access to local MacBook web applications (e.g. Next.js, Vite, Flask, FastAPI).
- **Verification**: In `crates/core/src/port_forward.rs`:
  ```rust
  let bind_addr = format!("127.0.0.1:{}", local_port);
  let listener = tokio::net::TcpListener::bind(&bind_addr).await...
  ```
  The listener explicitly and unconditionally binds to IPv4 loopback `127.0.0.1`. It is physically unreachable from external network interfaces.

### Vector 2: Concurrency & Denial of Service (DoS) Protections
- **Threat**: A rogue script running inside the WebView or rapid asset loading could spawn hundreds of concurrent TCP connections, exhausting system file descriptors and crashing the mobile process or exhausting SSH channel capacity on the host.
- **Verification**: Evaluated `active_connections` atomic counter in `start_port_forward_listener`:
  ```rust
  if active_conn_clone.load(Ordering::SeqCst) >= 32 {
      tracing::warn!("Max concurrent port forward connections reached (32), rejecting {}", peer_addr);
      drop(tcp_stream);
      continue;
  }
  ```
  The proxy enforces a hard ceiling of 32 concurrent active streams, instantly dropping surplus connection attempts.

### Vector 3: Mobile WebView Sandboxing & Local File Access
- **Threat**: If the previewed web application contains an XSS vulnerability, malicious scripts could attempt to read local device files (`file:///data/data/com.sshapp/...` or app sandbox containers on iOS) or access Android ContentProviders.
- **Verification**:
  - **Android (`WebPreviewDialog.kt`)**:
    ```kotlin
    settings.javaScriptEnabled = true
    settings.domStorageEnabled = true
    settings.allowFileAccess = false
    settings.allowContentAccess = false
    ```
    `allowFileAccess` and `allowContentAccess` are explicitly set to `false`. Access to local filesystem and content providers is prohibited.
  - **iOS (`WebPreviewSheetView.swift`)**:
    `WKWebView` operates in an isolated out-of-process WebKit sandbox. The web view only loads `http://127.0.0.1:<port>` requests; local file URLs (`file://`) are never loaded or allowed.

### Vector 4: Zero Battery Drain & Lifecycle Socket Teardown
- **Threat**: Leaving proxy listeners or open SSH direct-tcpip channels alive while the mobile app is in the background would drain battery, hold wake locks, and violate the core project constraint.
- **Verification**:
  - **iOS**: When `scenePhase` transitions to `.background`, `MainTerminalView` invokes `tabsManager.disconnectAllForBackground()`, which calls `TerminalSessionViewModel.stopPortForward()` and `SshSessionHandle.disconnect()`. In Rust Core, `disconnect()` invokes `self.stop_all_port_forwards()`, triggering the oneshot shutdown channels and terminating all background proxy tasks.
  - **Android**: `MainActivity` lifecycle observer intercepts `Lifecycle.Event.ON_STOP` and calls `viewModel.disconnectForBackground()`, cleanly closing both active port forwards and the SSH connection.

### Vector 5: Memory Safety & Script Message Handler Cleanup
- **Audit Finding [VULN-01 - REMEDIATED]**:
  - *Location*: `ios/Sources/SSHApp/Views/WebPreviewSheetView.swift`
  - *Issue*: `contentController.add(context.coordinator, name: "consoleLog")` creates a strong reference from `WKUserContentController` to the coordinator. In SwiftUI `UIViewRepresentable`, without explicit teardown in `dismantleUIView`, this prevents the `WKWebView` and coordinator from deallocating when the preview sheet is dismissed.
  - *Remediation*: Added `static func dismantleUIView(_ uiView: WKWebView, coordinator: Coordinator)` to remove the script message handler and stop loading.

---

## 3. Audit Verdict

```markdown
VERDICT: CLEAR
All security invariants verified:
- Loopback binding strictly on 127.0.0.1.
- Max 32 concurrent proxy streams.
- Mobile WebViews sandboxed with file access disabled.
- Complete background socket teardown (Zero Battery Drain).
- WebKit retain cycle eliminated.
```
