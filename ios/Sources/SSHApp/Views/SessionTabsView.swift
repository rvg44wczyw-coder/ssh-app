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
    /// Session actions (chat, tmux, permissions, agents) offered in the tab's long-press menu.
    public var isChatMode: ((TabSession) -> Bool)? = nil
    public var onSessionAction: ((UUID, AppAction) -> Void)? = nil

    @State private var tabToRename: TabSession? = nil
    @State private var tabForMenu: TabSession? = nil
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
        onClose: @escaping (UUID) -> Void,
        isChatMode: ((TabSession) -> Bool)? = nil,
        onSessionAction: ((UUID, AppAction) -> Void)? = nil
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
        self.isChatMode = isChatMode
        self.onSessionAction = onSessionAction
    }

    @ViewBuilder
    private func menuItems(for session: TabSession) -> some View {
        SessionMenuItems(
            session: session,
            isConnected: sessionStates[session.id] == .connected,
            isChatMode: isChatMode?(session) ?? false,
            canClose: sessions.count > 1,
            onAction: { onSessionAction?(session.id, $0) },
            onRename: {
                tabToRename = session
                renameText = session.title
                showRenameAlert = true
            },
            onClose: { onClose(session.id) }
        )
    }

    private func icon(for session: TabSession) -> (symbol: String, color: Color?) {
        if let provided = iconProvider?(session) { return provided }
        return (session.agentType.iconName, nil)
    }

    private var addMenu: some View {
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

    public var body: some View {
        tabStrip
    }

    private var tabStrip: some View {
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

                    Button(action: {
                        // A second tap on the selected tab opens its settings.
                        if session.id == selectedSessionId { tabForMenu = session } else { onSelect(session.id) }
                    }) {
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
                    .contextMenu { menuItems(for: session) }
                }

                addMenu
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
        }
        .background(Color.appBackground)
        .confirmationDialog(tabForMenu?.title ?? "", isPresented: Binding(get: { tabForMenu != nil }, set: { if !$0 { tabForMenu = nil } }), titleVisibility: .visible) {
            if let tab = tabForMenu { menuItems(for: tab) }
            Button("Cancel", role: .cancel) {}
        }
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

/// The one session menu, used by the tab strip and the cards on the main screen so both show the
/// same items: session actions, then Rename and Close.
struct SessionMenuItems: View {
    let session: TabSession
    let isConnected: Bool
    let isChatMode: Bool
    let canClose: Bool
    let onAction: (AppAction) -> Void
    let onRename: () -> Void
    let onClose: () -> Void

    var body: some View {
        ForEach(AppAction.allCases.filter(\.isSessionAction)) { action in
            Button { onAction(action) } label: {
                Label(action.title(isChatMode: isChatMode), systemImage: action.systemImage(isChatMode: isChatMode, keyVisible: false))
            }
            .disabled(action.requiresConnection && !isConnected)
        }
        Button(action: onRename) { Label("Rename", systemImage: "pencil") }
        if canClose {
            Button(role: .destructive, action: onClose) { Label("Close", systemImage: "xmark.circle") }
        }
    }
}
