import SwiftUI
import SshCoreBridge

public struct SessionTabsView: View {
    public let sessions: [TabSession]
    @Binding public var selectedSessionId: UUID
    public let sessionStates: [UUID: SessionState]
    public var serverNameProvider: ((TabSession) -> String?)? = nil
    public let onSelect: (UUID) -> Void
    public let onAddSession: () -> Void
    public let onRename: (UUID, String) -> Void
    public let onClose: (UUID) -> Void

    @State private var tabToRename: TabSession? = nil
    @State private var renameText: String = ""
    @State private var showRenameAlert: Bool = false

    public init(
        sessions: [TabSession],
        selectedSessionId: Binding<UUID>,
        sessionStates: [UUID: SessionState],
        serverNameProvider: ((TabSession) -> String?)? = nil,
        onSelect: @escaping (UUID) -> Void,
        onAddSession: @escaping () -> Void,
        onRename: @escaping (UUID, String) -> Void,
        onClose: @escaping (UUID) -> Void
    ) {
        self.sessions = sessions
        self._selectedSessionId = selectedSessionId
        self.sessionStates = sessionStates
        self.serverNameProvider = serverNameProvider
        self.onSelect = onSelect
        self.onAddSession = onAddSession
        self.onRename = onRename
        self.onClose = onClose
    }

    public var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 5) {
                ForEach(sessions) { session in
                    let isSelected = session.id == selectedSessionId
                    let state = sessionStates[session.id] ?? .disconnected
                    let serverName = serverNameProvider?(session)
                    let displayTitle: String = {
                        guard let serverName = serverName, !serverName.isEmpty else {
                            return session.title
                        }
                        if session.title.contains("(\(serverName))") {
                            return session.title
                        }
                        return "\(session.title) (\(serverName))"
                    }()

                    Button(action: { onSelect(session.id) }) {
                        HStack(spacing: 5) {
                            Image(systemName: session.agentType.iconName)
                                .font(.system(size: 10))

                            Text(displayTitle)
                                .font(.system(size: 11, weight: isSelected ? .bold : .medium))

                            // Status Indicator
                            Circle()
                                .fill(statusColor(for: state))
                                .frame(width: 5, height: 5)
                        }
                        .foregroundColor(isSelected ? .white : .primary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(
                            isSelected
                                ? Color.accentColor
                                : Color.appSecondaryBackground
                        )
                        .cornerRadius(6)
                    }
                    .contextMenu {
                        Button {
                            tabToRename = session
                            renameText = session.title
                            showRenameAlert = true
                        } label: {
                            Label("Rename Tab", systemImage: "pencil")
                        }

                        if sessions.count > 1 {
                            Button(role: .destructive) {
                                onClose(session.id)
                            } label: {
                                Label("Close Tab", systemImage: "xmark.circle")
                            }
                        }
                    }
                }

                // Add new tab button
                Button(action: onAddSession) {
                    Image(systemName: "plus")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(.accentColor)
                        .padding(5)
                        .background(Color.appSecondaryBackground)
                        .clipShape(Circle())
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
        }
        .background(Color.appBackground)
        .alert("Rename Tab", isPresented: $showRenameAlert) {
            TextField("Tab Name", text: $renameText)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                if let tab = tabToRename {
                    onRename(tab.id, renameText)
                }
            }
        } message: {
            Text("Enter a new title for this terminal tab:")
        }
    }

    private func statusColor(for state: SessionState) -> Color {
        switch state {
        case .connected: return .green
        case .connecting, .reconnecting: return .yellow
        case .failed: return .red
        case .disconnected: return .gray
        }
    }
}
