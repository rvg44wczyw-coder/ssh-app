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

    private var pin: String? { bundledHostHelperSha256() }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Host service", systemImage: "shippingbox").font(.headline)
            Text(summary).font(.caption).foregroundColor(.secondary)
            if let pin {
                Text("ssh-app-host · \(bundledHostHelperSize() / 1024) KB · sha256 \(pin.prefix(12))…")
                    .font(.system(size: 10, design: .monospaced)).foregroundColor(.secondary)
                Text("Installs to ~/.ssh-app/bin/ssh-app-host (mode 700). Nothing is started in the background.")
                    .font(.caption2).foregroundColor(.secondary)
            }
            if let error { Text(error).font(.caption).foregroundColor(.red) }
            if busy {
                ProgressView()
            } else if pin != nil, viewModel.sessionHandle != nil, viewModel.state == .connected {
                Button(installTitle) { Task { await install() } }
                    .buttonStyle(.bordered)
                    .disabled(isReady)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.appSecondaryBackground))
        .task(id: viewModel.state == .connected) { await refresh() }
    }

    private var isReady: Bool { if case .ready = status { return true }; return false }

    private var installTitle: String {
        switch status {
        case .absent?, nil: return "Install on this host"
        default: return "Update on this host"
        }
    }

    private var summary: String {
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
        guard let pin, let handle = viewModel.sessionHandle, viewModel.state == .connected else { return }
        status = try? await handle.hostHelperStatus(expectedSha256: pin)
    }

    private func install() async {
        guard let handle = viewModel.sessionHandle else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await handle.installBundledHostHelper()
        } catch {
            self.error = "Install failed: \(error.localizedDescription)"
        }
        await refresh()
    }
}
