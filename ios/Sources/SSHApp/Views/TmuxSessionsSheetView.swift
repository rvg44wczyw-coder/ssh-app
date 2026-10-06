import SwiftUI
import SshCoreBridge

public struct TmuxSessionsSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var serversManager: ServerProfilesManager
    @StateObject private var viewModel = TmuxSessionManagerViewModel()

    @State private var selectedServerId: UUID?
    @State private var sessionToKill: String?
    @State private var showKillConfirmation: Bool = false

    public let onAttachSession: (String, ServerProfile) -> Void

    public init(
        serversManager: ServerProfilesManager,
        initialServerId: UUID? = nil,
        onAttachSession: @escaping (String, ServerProfile) -> Void
    ) {
        self.serversManager = serversManager
        self._selectedServerId = State(initialValue: initialServerId ?? serversManager.activeProfile?.id)
        self.onAttachSession = onAttachSession
    }

    private var currentServer: ServerProfile? {
        serversManager.profile(for: selectedServerId)
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Server selector header if multiple profiles exist
                if serversManager.profiles.count > 1 {
                    HStack {
                        Text("Server:")
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                        Picker("Server", selection: Binding(
                            get: { selectedServerId ?? serversManager.activeProfile?.id ?? UUID() },
                            set: { newId in
                                selectedServerId = newId
                                refreshSessions()
                            }
                        )) {
                            ForEach(serversManager.profiles) { server in
                                Text(server.name).tag(server.id)
                            }
                        }
                        .pickerStyle(MenuPickerStyle())
                        Spacer()
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(Color(UIColor.secondarySystemBackground))
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
                                    .foregroundColor(.accentColor)

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
                                        .background(Color.accentColor)
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
                       let key = KeychainManager.shared.getPrivateKey() {
                        viewModel.killSession(name: name, server: server, privateKey: key)
                    }
                }
            } message: { name in
                Text("Are you sure you want to stop '\(name)'? Long-running agent tasks and programs inside this session will be terminated.")
            }
        }
    }

    private func refreshSessions() {
        guard let server = currentServer,
              let key = KeychainManager.shared.getPrivateKey(),
              !key.isEmpty else {
            return
        }
        viewModel.fetchSessions(server: server, privateKey: key)
    }

    private func formatTimestamp(_ epoch: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(epoch))
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}
