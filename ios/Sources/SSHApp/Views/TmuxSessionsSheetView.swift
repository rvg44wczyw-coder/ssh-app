import SwiftUI
import SshCoreBridge

public struct TmuxSessionsSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var serversManager: ServerProfilesManager
    @StateObject private var viewModel = TmuxSessionManagerViewModel()

    @State private var selectedServerId: UUID?
    @State private var sessionToKill: String?
    @State private var showKillConfirmation: Bool = false
    @State private var showServerPopup: Bool = false
    @State private var serverStatuses: [UUID: ServerReachability] = [:]

    public let onAttachSession: (String, ServerProfile) -> Void
    /// Claude config directories of the saved profiles (transcripts for context / usage).
    public let configDirs: [String]

    public init(
        serversManager: ServerProfilesManager,
        initialServerId: UUID? = nil,
        configDirs: [String] = [],
        onAttachSession: @escaping (String, ServerProfile) -> Void
    ) {
        self.serversManager = serversManager
        self._selectedServerId = State(initialValue: initialServerId ?? serversManager.activeProfile?.id)
        self.onAttachSession = onAttachSession
        self.configDirs = configDirs
    }

    private var currentServer: ServerProfile? {
        serversManager.profile(for: selectedServerId)
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Server selector header if multiple profiles exist
                if serversManager.profiles.count > 1 {
                    Button {
                        showServerPopup = true
                        Task {
                            await probeServers(serversManager.profiles) { id, state in
                                serverStatuses[id] = state
                            }
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Text("Server:")
                                .font(.subheadline)
                                .foregroundColor(.secondary)
                            Text(currentServer?.name ?? "Select")
                                .font(.subheadline.weight(.semibold))
                            Image(systemName: "chevron.up.chevron.down")
                                .font(.system(size: 11))
                                .foregroundColor(.secondary)
                            Spacer()
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(Color(UIColor.secondarySystemBackground))
                    .accessibilityLabel("Server: \(currentServer?.name ?? "none"). Change server")
                    Divider()
                }

                if let error = viewModel.errorMessage {
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.yellow)
                        Text(error)
                            .font(.caption)
                            .foregroundColor(.white)
                        Spacer()
                        Button(action: refreshSessions) {
                            Text("Retry")
                                .font(.caption.bold())
                                .foregroundColor(.white)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(Color.red.opacity(0.85))
                }

                if viewModel.isLoading && viewModel.sessions.isEmpty {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text("Querying remote tmux sessions...")
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if viewModel.sessions.isEmpty {
                    VStack(spacing: 14) {
                        Image(systemName: "terminal")
                            .font(.system(size: 48))
                            .foregroundColor(.secondary)
                        Text("No active tmux sessions")
                            .font(.headline)
                        Text("Start a terminal session or launch an agent to create one.")
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 32)
                        Button(action: refreshSessions) {
                            Label("Refresh", systemImage: "arrow.clockwise")
                                .font(.subheadline.bold())
                        }
                        .padding(.top, 8)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List {
                        ForEach(viewModel.sessions, id: \.name) { session in
                            HStack(spacing: 12) {
                                Image(systemName: "rectangle.stack.fill")
                                    .font(.title2)
                                    .foregroundColor(.appAccent)

                                VStack(alignment: .leading, spacing: 4) {
                                    HStack(spacing: 8) {
                                        Text(session.name)
                                            .font(.headline)
                                            .foregroundColor(.primary)

                                        if session.attached {
                                            Text("Attached")
                                                .font(.system(size: 10, weight: .bold))
                                                .padding(.horizontal, 6)
                                                .padding(.vertical, 2)
                                                .background(Color.green.opacity(0.2))
                                                .foregroundColor(.green)
                                                .cornerRadius(4)
                                        } else {
                                            Text("Detached")
                                                .font(.system(size: 10, weight: .medium))
                                                .padding(.horizontal, 6)
                                                .padding(.vertical, 2)
                                                .background(Color.secondary.opacity(0.2))
                                                .foregroundColor(.secondary)
                                                .cornerRadius(4)
                                        }
                                    }

                                    HStack(spacing: 6) {
                                        Text("\(session.windows) window\(session.windows == 1 ? "" : "s")")
                                            .font(.caption)
                                            .foregroundColor(.secondary)

                                        if session.createdTimestamp > 0 {
                                            Text("•")
                                                .font(.caption)
                                                .foregroundColor(.secondary)
                                            Text(formatTimestamp(session.createdTimestamp))
                                                .font(.caption)
                                                .foregroundColor(.secondary)
                                        }
                                    }
                                    if let u = viewModel.usage[session.name] {
                                        usageDetail(u)
                                    }
                                }

                                Spacer()

                                Button(action: {
                                    if let server = currentServer {
                                        onAttachSession(session.name, server)
                                        dismiss()
                                    }
                                }) {
                                    Text("Open")
                                        .font(.subheadline.bold())
                                        .padding(.horizontal, 10)
                                        .padding(.vertical, 5)
                                        .background(Color.appAccent)
                                        .foregroundColor(.white)
                                        .cornerRadius(8)
                                }
                                .buttonStyle(BorderlessButtonStyle())

                                Button(role: .destructive, action: {
                                    sessionToKill = session.name
                                    showKillConfirmation = true
                                }) {
                                    Image(systemName: "trash")
                                        .foregroundColor(.red)
                                }
                                .buttonStyle(BorderlessButtonStyle())
                                .padding(.leading, 6)
                            }
                            .padding(.vertical, 4)
                        }
                    }
                    .listStyle(InsetGroupedListStyle())
                    .safeAreaInset(edge: .bottom) { usageFooter }
                    .refreshable {
                        refreshSessions()
                    }
                }
            }
            .navigationTitle("Tmux Sessions")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button(action: refreshSessions) {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(viewModel.isLoading)
                }
            }
            .overlay {
                if showServerPopup {
                    ServerPickerPopup(
                        title: "Show tmux sessions of…",
                        subtitle: "Sessions are listed from the server you choose.",
                        servers: serversManager.profiles,
                        statuses: serverStatuses,
                        currentId: currentServer?.id,
                        onSelect: { server in
                            withAnimation(.easeOut(duration: 0.15)) { showServerPopup = false }
                            selectedServerId = server.id
                            refreshSessions()
                        },
                        onCancel: {
                            withAnimation(.easeOut(duration: 0.15)) { showServerPopup = false }
                        }
                    )
                }
            }
            .animation(.easeOut(duration: 0.15), value: showServerPopup)
            .onAppear {
                if selectedServerId == nil {
                    selectedServerId = serversManager.activeProfile?.id
                }
                refreshSessions()
            }
            .alert("Stop Session", isPresented: $showKillConfirmation, presenting: sessionToKill) { name in
                Button("Cancel", role: .cancel) {}
                Button("Stop & Remove", role: .destructive) {
                    if let server = currentServer,
                       let key = KeychainManager.shared.getPrivateKey(id: server.keyId) {
                        viewModel.killSession(name: name, server: server, privateKey: key)
                    }
                }
            } message: { name in
                Text("Are you sure you want to stop '\(name)'? Long-running agent tasks and programs inside this session will be terminated.")
            }
        }
    }

    private func tokens(_ n: UInt64) -> String {
        n >= 1_000_000 ? String(format: "%.1fM", Double(n) / 1e6) : n >= 1_000 ? String(format: "%.0fk", Double(n) / 1e3) : "\(n)"
    }

    private func bytes(_ n: UInt64) -> String { ByteCountFormatter.string(fromByteCount: Int64(n), countStyle: .memory) }

    private func clock(_ epoch: UInt64) -> String {
        Date(timeIntervalSince1970: TimeInterval(epoch)).formatted(date: .omitted, time: .shortened)
    }

    @ViewBuilder
    private func usageDetail(_ u: SessionUsage) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(u.agent ?? "model").font(.caption.bold()).foregroundColor(.mint)
                if let m = u.model { Text(m).font(.caption).foregroundColor(.secondary).lineLimit(1) }
            }
            if let used = u.contextUsed, let limit = u.contextLimit, limit > 0 {
                let frac = min(1, Double(used) / Double(limit))
                ProgressView(value: frac)
                    .tint(frac < 0.7 ? .green : (frac < 0.9 ? .orange : .red))
                Text("Context \(tokens(used)) / \(tokens(limit)) (\(Int(frac * 100))%)" + (frac >= 0.9 ? " — nearly full, consider /compact" : ""))
                    .font(.caption2).foregroundColor(frac < 0.9 ? .secondary : .red)
            } else if let o = u.ollama {
                Text("\(bytes(o.sizeBytes)) loaded, \(bytes(o.vramBytes)) in GPU memory" + (o.contextLength.map { ", context \($0)" } ?? "") + (o.expiresEpoch.map { ", unloads \(clock($0))" } ?? ""))
                    .font(.caption2).foregroundColor(.secondary)
            } else if u.agent == "ollama" {
                Text("Model is not loaded in memory right now").font(.caption2).foregroundColor(.secondary)
            } else if u.agent != "claude" {
                Text("No usage data for this agent").font(.caption2).foregroundColor(.secondary)
            }
            if let note = u.limitNote {
                Label(note.components(separatedBy: "|").first ?? note + (u.limitResetEpoch.map { " (resets \(clock($0)))" } ?? ""), systemImage: "exclamationmark.triangle.fill")
                    .font(.caption2).foregroundColor(.red)
                if let r = u.limitResetEpoch { Text("Resets \(clock(r))").font(.caption2).foregroundColor(.red) }
            }
        }
    }

    @ViewBuilder
    private var usageFooter: some View {
        if let r = viewModel.usageReport, r.fiveHour.messages > 0 || r.week.messages > 0 {
            VStack(alignment: .leading, spacing: 2) {
                Text("Claude tokens on this host (from transcripts, not your plan's remaining quota)")
                    .font(.caption2).foregroundColor(.secondary)
                Text("5 h: in \(tokens(r.fiveHour.input)) · out \(tokens(r.fiveHour.output)) · cache w \(tokens(r.fiveHour.cacheWrite)) / r \(tokens(r.fiveHour.cacheRead)) · \(r.fiveHour.messages) msgs")
                    .font(.caption2.monospacedDigit())
                Text("7 d: in \(tokens(r.week.input)) · out \(tokens(r.week.output)) · cache w \(tokens(r.week.cacheWrite)) / r \(tokens(r.week.cacheRead)) · \(r.week.messages) msgs" + (r.partial ? " (partial)" : ""))
                    .font(.caption2.monospacedDigit())
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16).padding(.vertical, 8)
            .background(.bar)
        } else if let e = viewModel.usageError {
            Text("Usage unavailable: \(e)").font(.caption2).foregroundColor(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.vertical, 6).background(.bar)
        }
    }

    private func refreshSessions() {
        guard let server = currentServer,
              let key = KeychainManager.shared.getPrivateKey(id: server.keyId),
              !key.isEmpty else {
            return
        }
        viewModel.fetchSessions(server: server, privateKey: key)
        viewModel.fetchUsage(server: server, privateKey: key, configDirs: configDirs)
    }

    private func formatTimestamp(_ epoch: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(epoch))
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}
