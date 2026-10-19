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

    private var target: HostTarget? { viewModel.hostTarget }
    private var isUnsupported: Bool { if case .unsupported? = target { return true }; return false }
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
            if isWsl || isUnsupported || isWindows {
                Button("WSL2 setup for Windows") { showWslSetup = true }.buttonStyle(.bordered)
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
            if isReady, viewModel.sessionHandle != nil, viewModel.state == .connected {
                agentSection
            }
            if let error { Text(error).font(.caption).foregroundColor(.red) }
            if busy {
                ProgressView()
            } else if pin != nil, !isUnsupported, !isWindows, viewModel.sessionHandle != nil, viewModel.state == .connected {
                Button(installTitle) { Task { await install() } }
                    .buttonStyle(.bordered)
                    .disabled(isReady)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.appSecondaryBackground))
        .task(id: viewModel.state == .connected) { await refresh() }
        .onDisappear { watch?.stop(); watch = nil }
        .sheet(isPresented: $showWslSetup) { WslSetupView(publicKey: viewModel.publicKeyLine) }
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
            .buttonStyle(.bordered)
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
