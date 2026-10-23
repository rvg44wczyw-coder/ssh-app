import SshCoreBridge
import SwiftUI

/// Ports the host listens on, with the program behind each. A tap opens the in-app web preview on
/// that port (through the SSH tunnel), so a dev server on the laptop is one tap away.
struct PortsSection: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var ports: [ListeningPort] = []
    @State private var loading = false
    @State private var failed = false
    @State private var preview: PreviewPort?

    private struct PreviewPort: Identifiable {
        let id = UUID()
        let port: UInt16
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Listening ports", systemImage: "network").font(.headline)
                Spacer()
                if loading { ProgressView() } else {
                    Button { Task { await load() } } label: { Image(systemName: "arrow.clockwise") }
                        .accessibilityLabel("Refresh ports")
                }
            }
            if failed {
                Text("The host did not answer.").font(.footnote).foregroundColor(.secondary)
            } else if ports.isEmpty && !loading {
                Text("Nothing is listening, or the host could not say.").font(.footnote).foregroundColor(.secondary)
            }
            GlassEffectContainer(spacing: 8) {
                VStack(spacing: 8) {
                    ForEach(ports, id: \.port) { p in row(p) }
                }
            }
            Text("Open a port in the preview to see a dev server of the host; a port shown with a globe is reachable from other machines too.")
                .font(.caption2).foregroundColor(.secondary)
        }
        .task { await load() }
        .sheet(item: $preview) { target in WebPreviewSheetView(viewModel: viewModel, port: target.port) }
    }

    private func row(_ p: ListeningPort) -> some View {
        Button { preview = PreviewPort(port: p.port) } label: {
            HStack(spacing: 10) {
                Image(systemName: p.localOnly ? "lock.fill" : "globe")
                    .foregroundColor(p.localOnly ? .secondary : .orange)
                    .accessibilityLabel(p.localOnly ? "Local only" : "Reachable from other machines")
                Text(verbatim: String(p.port)).font(.system(size: 16, weight: .semibold, design: .monospaced))
                Text(verbatim: p.process.isEmpty ? "—" : p.process).font(.subheadline).foregroundColor(.secondary).lineLimit(1)
                Spacer()
                if let pid = p.pid { Text(verbatim: "pid \(pid)").font(.caption2).foregroundColor(.secondary) }
                Image(systemName: "safari").foregroundColor(.accentColor)
            }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: RoundedRectangle(cornerRadius: 14))
    }

    private func load() async {
        loading = true
        defer { loading = false }
        if let found = await viewModel.fetchListeningPorts() {
            ports = found
            failed = false
        } else {
            failed = true
        }
    }
}
