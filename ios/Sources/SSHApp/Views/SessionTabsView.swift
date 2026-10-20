import SwiftUI
import SshCoreBridge

public struct SessionTabsView: View {
    public let sessions: [TabSession]
    public let attention: ((TabSession) -> SessionAttention?)?
    let onRestart: ((UUID) -> Void)?
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
    public var onSetScheme: ((UUID, String?) -> Void)? = nil

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
        attention: ((TabSession) -> SessionAttention?)? = nil,
        onRestart: ((UUID) -> Void)? = nil,
        onSessionAction: ((UUID, AppAction) -> Void)? = nil,
        onSetScheme: ((UUID, String?) -> Void)? = nil
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
        self.attention = attention
        self.onRestart = onRestart
        self.onSessionAction = onSessionAction
        self.onSetScheme = onSetScheme
    }

    /// The tab's dropdown: the session actions as icon tiles (same look as the quick menu), then
    /// Theme, Rename and Close.
    private func sessionDropdown(for session: TabSession) -> some View {
        VStack(spacing: 10) {
            ActionPanelView(
                scope: .session,
                isChatMode: isChatMode?(session) ?? false,
                keyVisible: false,
                isConnected: sessionStates[session.id] == .connected,
                onSelect: { action in
                    tabForMenu = nil
                    onSessionAction?(session.id, action)
                }
            )
            Divider()
            // Theme, Rename, Restart, Close: laid out like the tiles above (same columns and icon
            // size), so a narrower or wider menu keeps them in step.
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: dropdownColumns), spacing: 8) {
                if let onSetScheme {
                    Menu {
                        Button { onSetScheme(session.id, nil) } label: {
                            Label("Automatic", systemImage: session.schemeId == nil ? "checkmark" : "wand.and.stars")
                        }
                        ForEach(TerminalScheme.all) { sch in
                            Button { onSetScheme(session.id, sch.id) } label: {
                                if session.schemeId == sch.id { Label(sch.name, systemImage: "checkmark") } else { Text(sch.name) }
                            }
                        }
                    } label: { dropdownButton("Theme", "paintpalette", .primary) }
                }
                Button {
                    tabForMenu = nil
                    tabToRename = session
                    renameText = session.title
                    showRenameAlert = true
                } label: { dropdownButton("Rename", "pencil", .primary) }
                if let onRestart, session.profileId != nil || session.command != nil {
                    Button {
                        tabForMenu = nil
                        onRestart(session.id)
                    } label: { dropdownButton("Restart", "arrow.clockwise", .orange) }
                    .disabled(sessionStates[session.id] != .connected)
                }
                if sessions.count > 1 {
                    Button(role: .destructive) {
                        tabForMenu = nil
                        onClose(session.id)
                    } label: { dropdownButton("Close", "xmark.circle", .red) }
                }
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 14)
            .padding(.bottom, 12)
        }
        .frame(width: MenuGridPrefs.panelWidth(columns: dropdownColumns, iconSize: sessionIconSize))
    }

    @AppStorage(MenuGridPrefs.columnsKey(session: true)) private var sessionColumns = MenuGridPrefs.defaultColumns
    @AppStorage(MenuGridPrefs.sizeKey(session: true)) private var sessionIconSize = MenuGridPrefs.defaultSize
    @Environment(\.dynamicTypeSize) private var dropdownTypeSize

    private var dropdownColumns: Int {
        MenuGridPrefs.shownColumns(sessionColumns, accessibility: dropdownTypeSize.isAccessibilitySize)
    }

    private func dropdownButton(_ title: String, _ icon: String, _ color: Color) -> some View {
        VStack(spacing: 4) {
            Image(systemName: icon).font(.system(size: sessionIconSize * 0.42))
            Text(title).font(.caption2).lineLimit(1)
        }
        .foregroundColor(color)
        .frame(maxWidth: .infinity, minHeight: CGFloat(sessionIconSize) + 12)
        .background(Color.appSecondaryBackground, in: RoundedRectangle(cornerRadius: 12))
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
            // Fixed-size label with the circle inside it: the menu lifts and re-places its label
            // while opening, and a glyph padded inside a flexible label slid off-centre.
            ZStack {
                Circle().fill(Color.appSecondaryBackground)
                Image(systemName: "plus")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.appAccent)
            }
            .frame(width: 22, height: 22)
            .contentShape(Circle())
        } primaryAction: {
            onAddSession()
        }
        .fixedSize()
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

                            if let a = attention?(session) {
                                SessionAttentionBadge(attention: a, size: 10)
                            }
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
                    // Long press opens the same dropdown as a second tap on the selected tab.
                    .simultaneousGesture(LongPressGesture(minimumDuration: 0.4).onEnded { _ in tabForMenu = session })
                    .popover(isPresented: Binding(
                        get: { tabForMenu?.id == session.id },
                        set: { if !$0 { tabForMenu = nil } }
                    ), arrowEdge: .top) {
                        sessionDropdown(for: session)
                            .presentationCompactAdaptation(.popover)
                    }
                }

                addMenu
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

/// The one session menu, used by the tab strip and the cards on the main screen so both show the
/// same items: session actions, then Rename and Close.
struct SessionMenuItems: View {
    let session: TabSession
    let isConnected: Bool
    let isChatMode: Bool
    let canClose: Bool
    let onAction: (AppAction) -> Void
    var onScheme: ((String?) -> Void)? = nil
    let onRename: () -> Void
    var onRestart: (() -> Void)? = nil
    let onClose: () -> Void
    @AppStorage(QuickMenuPrefs.orderKey) private var orderRaw: String = ""
    @AppStorage(QuickMenuPrefs.storageKey) private var hiddenRaw: String = ""

    var body: some View {
        ForEach(QuickMenuPrefs.ordered(from: orderRaw).filter { $0.isSessionAction && (!$0.onlyInFeed || isChatMode) && !QuickMenuPrefs.hidden(from: hiddenRaw).contains($0) }) { action in
            Button { onAction(action) } label: {
                Label(action.title(isChatMode: isChatMode), systemImage: action.systemImage(isChatMode: isChatMode, keyVisible: false))
            }
            .disabled(action.requiresConnection && !isConnected)
        }
        if let onScheme {
            Menu {
                Button { onScheme(nil) } label: {
                    Label("Automatic", systemImage: session.schemeId == nil ? "checkmark" : "wand.and.stars")
                }
                ForEach(TerminalScheme.all) { sch in
                    Button { onScheme(sch.id) } label: {
                        if session.schemeId == sch.id { Label(sch.name, systemImage: "checkmark") } else { Text(sch.name) }
                    }
                }
            } label: { Label("Theme", systemImage: "paintpalette") }
        }
        Button(action: onRename) { Label("Rename", systemImage: "pencil") }
        if let onRestart {
            Button(action: onRestart) { Label("Restart", systemImage: "arrow.clockwise") }
                .disabled(!isConnected)
        }
        if canClose {
            Button(role: .destructive, action: onClose) { Label("Close", systemImage: "xmark.circle") }
        }
    }
}
