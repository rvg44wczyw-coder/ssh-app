import SwiftUI
import SshCoreBridge

/// State and install of the typed host helper (`ssh-app-host`, feature 24). The helper is embedded
/// in the app; installing writes it to `~/.ssh-app/bin` on the host and the app only ever runs a
/// host file whose SHA-256 equals the embedded one. Without it the usage line uses the Python probe.
struct HostHelperCard: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var status: HostHelperStatus?
    @State private var busy = false
    @State private var error: String?
    @State private var agent: HostAgentState?
    @State private var agentChecked = false
    @State private var watch: HostAgentWatch?

    @State private var pin: String?
    @State private var showWslSetup = false
    /// The hook script on the host is somebody else's file: asked before it is replaced.
    @State private var askReplaceHook = false
    @State private var hookState: HookScriptState?

    private var target: HostTarget? { viewModel.hostTarget }
    private var isUnsupported: Bool { if case .unsupported? = target { return true }; return false }
    /// Unsupported because it looks like Windows without WSL2 (not an unknown CPU or a probe that failed).
    private var unsupportedForWindows: Bool {
        if case .unsupported(let reason)? = target { return reason.hasPrefix("Windows") }
        return false
    }
    private var isWindows: Bool { target.map { hostTargetMultiplexer(target: $0) == .windowsMux } ?? false }
    private var isWsl: Bool {
        switch target {
        case .wslX8664?, .wslAarch64?: return true
        default: return false
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Host service", systemImage: "shippingbox").font(.headline)
            if let target {
                Text(hostTargetLabel(target: target)).font(.caption.bold())
            }
            Text(summary).font(.caption).foregroundColor(.secondary)
            if case .unsupported(let reason)? = target, !unsupportedForWindows {
                Text(reason).font(.caption).foregroundColor(.orange)
            }
            if isWsl || isWindows || unsupportedForWindows {
                Button("WSL2 setup for Windows") { showWslSetup = true }.buttonStyle(.glass)
            }
            if isWindows {
                Text("Windows host: the app installs and verifies its helper (the session multiplexer) itself when a tab connects.")
                    .font(.caption2).foregroundColor(.secondary)
            }
            if let pin, !isUnsupported, !isWindows {
                Text("ssh-app-host · \((target.map { bundledHostHelperSizeFor(target: $0) } ?? bundledHostHelperSize()) / 1024) KB · sha256 \(pin.prefix(12))…")
                    .font(.system(size: 10, design: .monospaced)).foregroundColor(.secondary)
                Text("Installs to ~/.ssh-app/bin/ssh-app-host (mode 700). Nothing runs in the background until you check the agent below.")
                    .font(.caption2).foregroundColor(.secondary)
            }
            if isReady, let hookState {
                Label(hookStateText(hookState), systemImage: hookState == .rust ? "checkmark.seal" : "exclamationmark.triangle")
                    .font(.caption2)
                    .foregroundColor(hookState == .rust ? .secondary : .orange)
            }
            if isReady, viewModel.sessionHandle != nil, viewModel.state == .connected {
                agentSection
                PushSection(viewModel: viewModel)
                HostCheckSection(viewModel: viewModel)
            }
            if let error { Text(error).font(.caption).foregroundColor(.red) }
            if busy {
                ProgressView()
            } else if pin != nil, !isUnsupported, !isWindows, viewModel.sessionHandle != nil, viewModel.state == .connected {
                Button(installTitle) { Task { await install() } }
                    .buttonStyle(.glass)
                    .disabled(isReady)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .glassCard(cornerRadius: 18)
        .task(id: viewModel.state == .connected) { await refresh() }
        .onDisappear { watch?.stop(); watch = nil }
        .sheet(isPresented: $showWslSetup) { WslSetupView(publicKey: viewModel.publicKeyLine) }
        .alert("Replace the hook script?", isPresented: $askReplaceHook) {
            Button("Replace", role: .destructive) { Task { await refreshHookScript(overwriteForeign: true) } }
            Button("Keep it", role: .cancel) {}
        } message: {
            Text("~/.ssh-app/bin/ssh-app-hook on this host is not ssh-app's own script. The agents' hooks only use the new helper with ssh-app's script. Replace it?")
        }
    }

    private func hookStateText(_ state: HookScriptState) -> String {
        switch state {
        case .rust: return "Agent hooks run this helper (Rust)."
        case .python: return "Agent hooks run the Python script: its pin is not this helper."
        case .foreign: return "~/.ssh-app/bin/ssh-app-hook is not ssh-app's script."
        case .notInstalled: return "Agent hooks are not installed on this host."
        }
    }

    /// The pin of the helper sits inside the hook script; with an old pin every hook falls back to
    /// the Python code. Our own script is refreshed silently, any other file only after a yes.
    private func refreshHookScript(overwriteForeign: Bool) async {
        guard let handle = viewModel.sessionHandle else { return }
        do {
            if try await handle.refreshHookScript(overwriteForeign: overwriteForeign) == .needsConfirmation {
                askReplaceHook = true
            }
            hookState = try? await handle.hookScriptState()
        } catch {
            self.error = "Could not update the hook script: \(error.localizedDescription)"
        }
    }

    @ViewBuilder
    private var agentSection: some View {
        Divider()
        Text("Agent").font(.subheadline.bold())
        Text("Starts a small process on the host that tracks your agents' state (approval, finished, waiting). It exits by itself 12 hours after the last contact from this phone and uses no CPU while idle.")
            .font(.caption2).foregroundColor(.secondary)
        if let agent {
            Text(agent.sessions.isEmpty ? "No agent sessions seen yet." : "\(agent.sessions.count) session(s), \(agent.pendingApprovals) awaiting approval")
                .font(.caption)
            ForEach(Array(agent.sessions.enumerated()), id: \.offset) { _, s in
                Text("\(s.agent) · \(s.session) · \(s.state)")
                    .font(.system(size: 11, design: .monospaced))
            }
        } else if agentChecked {
            Text("The agent did not answer.").font(.caption).foregroundColor(.orange)
        }
        if watch != nil {
            Label("Live: changes appear as they happen", systemImage: "dot.radiowaves.left.and.right")
                .font(.caption2).foregroundColor(.green)
        }
        Button(agentChecked ? "Refresh agent state" : "Check agent") { Task { await checkAgent() } }
            .buttonStyle(.glass)
    }

    private func checkAgent() async {
        guard let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        agent = try? await handle.hostAgentState(expectedSha256: pin)
        agentChecked = true
        if agent != nil, watch == nil { await startWatch(handle, pin: pin) }
    }

    /// One exec channel while this card is on screen; each change re-reads the state. It closes
    /// with the card and with the connection, so nothing runs in the background.
    private func startWatch(_ handle: SshSessionHandle, pin: String) async {
        let listener = AgentEvents(
            onChange: { Task { @MainActor in agent = try? await handle.hostAgentState(expectedSha256: pin) } },
            onClosed: { Task { @MainActor in watch = nil } }
        )
        watch = try? await handle.watchHostAgent(expectedSha256: pin, listener: listener)
    }

    private final class AgentEvents: HostAgentListener, @unchecked Sendable {
        let onChange: @Sendable () -> Void
        let onClosedHandler: @Sendable () -> Void
        init(onChange: @escaping @Sendable () -> Void, onClosed: @escaping @Sendable () -> Void) {
            self.onChange = onChange
            self.onClosedHandler = onClosed
        }
        func onEvent(event: HostAgentEvent) { onChange() }
        func onClosed() { onClosedHandler() }
    }

    private var isReady: Bool { if case .ready = status { return true }; return false }

    private var installTitle: String {
        switch status {
        case .absent?, nil: return "Install on this host"
        default: return "Update on this host"
        }
    }

    private var summary: String {
        if case .unsupported(let reason)? = target { return reason }
        guard pin != nil else { return "This build has no host helper embedded; usage uses the Python probe." }
        switch status {
        case nil: return viewModel.state == .connected ? "Checking…" : "Connect to a host to check it."
        case .absent?: return "Not installed. Usage uses the Python probe, which needs python3 on the host."
        case .mismatch?: return "A different ssh-app-host is installed (older version or changed). It is not run; update it."
        case .unusable?: return "Installed but it does not run on this host (wrong platform?). The Python probe is used."
        case .ready(let version)?: return "Installed and verified, version \(version)."
        }
    }

    private func refresh() async {
        guard let handle = viewModel.sessionHandle, viewModel.state == .connected else { return }
        pin = await viewModel.helperPin()
        guard let pin, !isUnsupported, !isWindows else { return }
        status = try? await handle.hostHelperStatus(expectedSha256: pin)
        if case .ready? = status { await refreshHookScript(overwriteForeign: false) }
    }

    private func install() async {
        guard let handle = viewModel.sessionHandle else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            if let target {
                try await handle.installBundledHostHelperFor(target: target)
            } else {
                try await handle.installBundledHostHelper()
            }
        } catch {
            self.error = "Install failed: \(error.localizedDescription)"
        }
        await refresh()
    }
}

/// Ready-to-copy commands that turn a Windows PC into a host through WSL2 (feature 33).
struct WslSetupView: View {
    let publicKey: String?
    @State private var windowsNative = false
    @State private var route: WslNetworkRoute = .tailscaleInDistro
    @State private var port = "22"
    @Environment(\.dismiss) private var dismiss

    private var snippets: [WslSnippet] {
        if windowsNative, let publicKey { return (try? windowsSetupSnippets(publicKey: publicKey)) ?? [] }
        guard let publicKey, let p = UInt16(port) else { return [] }
        return (try? wslSetupSnippets(route: route, publicKey: publicKey, sshPort: p)) ?? []
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("tmux does not run natively on Windows. Use WSL2: sshd, tmux and the helper run inside a Linux distro on the PC. Do these steps once.")
                        .font(.caption)
                    Picker("Setup", selection: $windowsNative) {
                        Text("WSL2 (recommended)").tag(false)
                        Text("Windows directly").tag(true)
                    }
                    .pickerStyle(.segmented)
                    if windowsNative {
                        Text("Terminal tabs, session list and end work. Agent feed, git, usage and the file browser need the POSIX shell and are not available yet.")
                            .font(.caption2).foregroundColor(.orange)
                    }
                    if !windowsNative { Picker("Network", selection: $route) {
                        Text("Tailscale in WSL").tag(WslNetworkRoute.tailscaleInDistro)
                        Text("Mirrored networking").tag(WslNetworkRoute.mirroredNetworking)
                        Text("Port forward").tag(WslNetworkRoute.portProxy)
                    } }
                    if !windowsNative, route != .tailscaleInDistro {
                        HStack {
                            Text("Windows port")
                            TextField("22", text: $port).keyboardType(.numberPad).multilineTextAlignment(.trailing)
                        }
                    }
                }
                if snippets.isEmpty {
                    Section { Text("This tab's public key is not available, or the port is not valid.").foregroundColor(.orange) }
                }
                ForEach(Array(snippets.enumerated()), id: \.offset) { index, snippet in
                    Section("\(index + 1). \(snippet.title)") {
                        Text(snippet.whereToRun).font(.caption).foregroundColor(.secondary)
                        Text(snippet.body).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                        Text(snippet.note).font(.caption2).foregroundColor(.secondary)
                        Button("Copy") { UIPasteboard.general.string = snippet.body }
                    }
                }
            }
            .navigationTitle("WSL2 setup")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }
}


/// Push from this host (feature 35): the phone's token, the relay and a test.
private struct PushSection: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @ObservedObject private var push = PushManager.shared
    @State private var url = PushManager.shared.relayURL
    @State private var secret = PushManager.shared.relaySecret
    @State private var status: HostPushStatus?
    @State private var message: String?
    @State private var busy = false

    var body: some View {
        Divider()
        Text("Push").font(.subheadline.bold())
        Text("A notification when an agent needs approval, finishes, waits or a sandbox blocks a host, while the app is closed. What travels through your relay (scripts/apns-worker) and Apple is a fixed sentence: no commands, paths or names. The tab name and, for a blocked host, the host name are added on this phone only, so they show on the lock screen. To hide them there set Settings → Notifications → Show Previews to When Unlocked or Never. Works only while the host agent runs.")
            .font(.caption2).foregroundColor(.secondary)
        if push.token == nil {
            Button("Allow notifications") { Task { await push.enable() } }.buttonStyle(.glass)
        } else {
            Text("Notifications allowed · \(PushManager.apsEnvironment)").font(.caption).foregroundColor(.green)
        }
        TextField("Relay address (https://…/push)", text: $url)
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .font(.system(size: 12, design: .monospaced))
            .onChange(of: url) { _, v in push.relayURL = v }
        SecureField("Relay secret", text: $secret)
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .font(.system(size: 12, design: .monospaced))
            .onChange(of: secret) { _, v in push.relaySecret = v }
        if let status {
            Text(status.registered ? "Registered on this host (\(status.env), …\(status.tokenTail))" : "Not registered on this host")
                .font(.caption)
        }
        if let message { Text(message).font(.caption).foregroundColor(.orange) }
        if busy {
            ProgressView()
        } else {
            HStack {
                Button("Register this host") { Task { await register() } }
                    .disabled(push.token == nil || url.isEmpty || secret.isEmpty)
                Button("Test") { Task { await run { try await $0.hostPushTest(expectedSha256: $1) } } }
                    .disabled(status?.registered != true)
                Button("Remove", role: .destructive) {
                    push.forget(viewModel.hostCacheKey)
                    Task { await run { try await $0.hostPushUnregister(expectedSha256: $1) } }
                }
                    .disabled(status?.registered != true)
            }
            .buttonStyle(.glass)
        }
        if let e = push.lastError { Text(e).font(.caption2).foregroundColor(.red) }
        Color.clear.frame(height: 0).task { await refresh() }
    }

    private func refresh() async {
        guard let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        status = try? await handle.hostPushStatus(expectedSha256: pin)
    }

    private func register() async {
        guard let token = push.token else { return }
        await run { try await $0.hostPushRegister(expectedSha256: $1, token: token, env: PushManager.apsEnvironment, relayUrl: url, relaySecret: secret) }
        if status?.registered == true { push.markRegistered(viewModel.hostCacheKey) }
    }

    private func run(_ call: (SshSessionHandle, String) async throws -> HostPushResult) async {
        guard let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        busy = true
        defer { busy = false }
        do {
            let r = try await call(handle, pin)
            message = r.ok ? nil : r.message
        } catch {
            message = error.localizedDescription
        }
        await refresh()
    }
}

/// "Check this host" (feature 44): the helper's read-only `doctor` reports facts about the host and
/// the core turns them into suggestions. Nothing is changed by looking.
struct HostCheckSection: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var doctor: HostDoctor?
    @State private var findings: [OnboardingFinding] = []
    @State private var busy = false
    @State private var failed = false

    var body: some View {
        Divider()
        Text("Host check").font(.subheadline.bold())
        Text("Looks at which tools and agents are on this host. It only reads; nothing is changed.")
            .font(.caption2).foregroundColor(.secondary)
        if let doctor {
            Text(summary(doctor)).font(.system(size: 11, design: .monospaced)).foregroundColor(.secondary)
            if findings.isEmpty {
                Label("Nothing to do", systemImage: "checkmark.circle").font(.caption).foregroundColor(.green)
            }
            ForEach(findings, id: \.id) { f in
                VStack(alignment: .leading, spacing: 2) {
                    Label(f.title, systemImage: symbol(f.level)).font(.caption.bold()).foregroundColor(color(f.level))
                    Text(f.detail).font(.caption2).foregroundColor(.secondary)
                }
            }
        } else if failed {
            Text("The helper did not answer.").font(.caption).foregroundColor(.orange)
        }
        if busy {
            ProgressView()
        } else {
            Button(doctor == nil ? "Check this host" : "Check again") { Task { await run() } }
                .buttonStyle(.glass)
        }
    }

    private func summary(_ d: HostDoctor) -> String {
        func tool(_ name: String, _ t: DoctorTool) -> String {
            t.installed ? "\(name) \(t.version)".trimmingCharacters(in: .whitespaces) : "no \(name)"
        }
        var parts = [d.os + " " + d.arch, tool("tmux", d.tmux), tool("git", d.git), tool("python3", d.python3)]
        parts.append(d.tailscale.installed ? "tailscale " + (d.tailscaleIp.isEmpty ? "on" : d.tailscaleIp) : "no tailscale")
        if d.claudeFound { parts.append("claude") }
        if d.antigravityFound { parts.append("agy") }
        if d.ollamaInstalled { parts.append(d.ollamaRunning ? "ollama (running)" : "ollama (stopped)") }
        return parts.joined(separator: " · ")
    }

    private func symbol(_ level: FindingLevel) -> String {
        switch level {
        case .needed: return "exclamationmark.triangle"
        case .suggested: return "lightbulb"
        case .info: return "info.circle"
        }
    }

    private func color(_ level: FindingLevel) -> Color {
        switch level {
        case .needed: return .orange
        case .suggested: return .accentColor
        case .info: return .secondary
        }
    }

    private func run() async {
        guard let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        busy = true; failed = false
        defer { busy = false }
        do {
            if let d = try await handle.hostDoctor(expectedSha256: pin) {
                doctor = d
                findings = onboardingFindings(doctor: d)
            } else {
                failed = true
            }
        } catch {
            failed = true
        }
    }
}
