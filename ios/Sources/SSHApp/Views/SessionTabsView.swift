import SwiftUI
import SshCoreBridge

public struct SessionTabsView: View {
    public let sessions: [TabSession]
    @Binding public var selectedSessionId: UUID
    public let sessionStates: [UUID: SessionState]
    public var serverNameProvider: ((TabSession) -> String?)? = nil
    /// Icon (SF Symbol) and optional color of a tab; defaults to the agent type's icon.
    public var iconProvider: ((TabSession) -> (symbol: String, color: Color?))? = nil
    public let onSelect: (UUID) -> Void
    public let onAddSession: () -> Void
    /// Presets listed in the long-press menu of the + button (id, title, detail).
    public var presets: [(id: String, title: String, detail: String)] = []
    public var onLaunchPreset: ((String) -> Void)? = nil
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
        iconProvider: ((TabSession) -> (symbol: String, color: Color?))? = nil,
        onSelect: @escaping (UUID) -> Void,
        onAddSession: @escaping () -> Void,
        presets: [(id: String, title: String, detail: String)] = [],
        onLaunchPreset: ((String) -> Void)? = nil,
        onRename: @escaping (UUID, String) -> Void,
        onClose: @escaping (UUID) -> Void
    ) {
        self.sessions = sessions
        self._selectedSessionId = selectedSessionId
        self.sessionStates = sessionStates
        self.serverNameProvider = serverNameProvider
        self.iconProvider = iconProvider
        self.onSelect = onSelect
        self.onAddSession = onAddSession
        self.presets = presets
        self.onLaunchPreset = onLaunchPreset
        self.onRename = onRename
        self.onClose = onClose
    }

    private func icon(for session: TabSession) -> (symbol: String, color: Color?) {
        if let provided = iconProvider?(session) { return provided }
        return (session.agentType.iconName, nil)
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
                            ProfileIconView(ref: icon(for: session).symbol, size: 10)
                                .foregroundColor(isSelected ? .white : (icon(for: session).color ?? .primary))

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
                                ? Color.appAccent
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
                Menu {
                    ForEach(presets, id: \.id) { p in
                        Button { onLaunchPreset?(p.id) } label: {
                            Text(p.detail.isEmpty ? p.title : "\(p.title) — \(p.detail)")
                        }
                    }
                    Button { onAddSession() } label: { Label("New tab", systemImage: "plus") }
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(.appAccent)
                        .padding(5)
                        .background(Color.appSecondaryBackground)
                        .clipShape(Circle())
                } primaryAction: {
                    onAddSession()
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
