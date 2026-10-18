import SwiftUI
import SshCoreBridge

public struct MainTerminalView: View {
    @Environment(\.scenePhase) private var scenePhase

    @StateObject private var serversManager = ServerProfilesManager()
    @StateObject private var tabsManager = TerminalTabsManager()
    @StateObject private var agentProfiles = AgentProfilesManager()

    private enum ServerPickerContext {
        case startup
        case newTab
    }

    private enum ActiveModalSheet: String, Identifiable {
        case settings
        case publicKey
        case tmuxSessions
        case diffViewer
        case webPreview
        case aiAssistant
        case snippets
        case commands
        case sessionPermissions
        case telemetry
        case fileBrowser
        case agentProfiles

        var id: String { rawValue }
    }

    @State private var isCtrlLocked: Bool = false
    @AppStorage(AppAccent.storageKey) private var accentRaw: String = AppAccent.blue.rawValue

    /// Profile override, else the scheme chosen for the tab's agent, else the default one.
    private var activeScheme: TerminalScheme {
        guard let tab = tabsManager.activeSession else { return .fallback }
        let profile = agentProfiles.profiles.first { $0.id == tab.profileId }
        let slot = profile.map { SchemeSlot.forAgent($0.agent) } ?? SchemeSlot.forTab(tab.agentType)
        return TerminalSchemes.resolve(profileSchemeId: profile?.terminalSchemeId, slot: slot)
    }
    @State private var activeSheet: ActiveModalSheet? = nil
    /// The AI assistant is a full screen of its own, not a card over the terminal.
    @State private var showAssistant = false
    /// Session the assistant talks through (its connection reaches the host model) and the server's name for the title.
    @State private var assistantVM: TerminalSessionViewModel?
    @State private var assistantServerName = ""
    @StateObject private var approvals = ApprovalsCenter()

    @ObservedObject private var subscriptions = SubscriptionManager.shared
    @State private var showPaywall: Bool = false

    @State private var showServerPicker: Bool = false
    @State private var showActionPanel: Bool = false
    /// Main screen is the list of session cards; opening a card shows the terminal.
    @State private var showHome: Bool = true
    @StateObject private var usageMonitor = SessionUsageMonitor()
    @AppStorage("usage_strip_enabled") private var usageStripEnabled: Bool = true
    @State private var serverStatuses: [UUID: ServerReachability] = [:]
    @State private var pingingServers: [ServerProfile] = []
    @State private var serverPickerContext: ServerPickerContext = .startup
    @State private var isCheckingServers: Bool = false

    public init() {}

    private func dismissKeyboard() {
        #if canImport(UIKit)
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        #endif
    }

    @ViewBuilder
    private var approvalViews: some View {
                ForEach(approvals.notices) { notice in
                    ApprovalNoticeView(entry: notice) {
                        Task { await approvals.dismiss(notice) }
                    }
                }
                if let approval = approvals.current {
                    ApprovalBannerView(
                        entry: approval,
                        queuedBehind: approvals.queuedBehindCurrent,
                        isBusy: approvals.busyIds.contains(approval.id),
                        errorMessage: approvals.errors[approval.id],
                        onDecision: { approved in
                            Task { await approvals.decide(approval, approved: approved) }
                        }
                    )
                }
    }

    private func refreshUsage(force: Bool = false) {
        guard usageStripEnabled, scenePhase == .active,
              let session = tabsManager.activeSession,
              tabsManager.activeViewModel?.state == .connected else { return }
        let server = serversManager.profile(for: session.serverId) ?? serversManager.activeProfile
        usageMonitor.refresh(
            sessionName: session.sessionName,
            server: server,
            configDirs: agentProfiles.profiles.filter { $0.agent == .claudeCode }.compactMap { $0.configDir },
            force: force
        )
    }

    private func presentSheet(_ sheet: ActiveModalSheet) {
        dismissKeyboard()
        if sheet == .aiAssistant {
            guard let vm = tabsManager.activeViewModel else { return }
            let server = serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile
            openAssistant(through: vm, serverName: server?.name ?? "")
            return
        }
        activeSheet = sheet
    }

    public var body: some View {
        NavigationStack {
            Group {
            if showHome {
                VStack(spacing: 0) {
                approvalViews
                SessionsHomeView(
                    sessions: tabsManager.sessions,
                    states: tabsManager.sessionStates,
                    viewModel: { tabsManager.viewModel(for: $0) },
                    serverName: { serversManager.profile(for: $0.serverId)?.name ?? serversManager.activeProfile?.name },
                    icon: { session in
                        let profile = agentProfiles.profiles.first { $0.id == session.profileId }
                        return (ProfileIcons.symbol(profile, tabType: session.agentType), ProfileIcons.color(profile))
                    },
                    scheme: { session in
                        let profile = agentProfiles.profiles.first { $0.id == session.profileId }
                        let slot = profile.map { SchemeSlot.forAgent($0.agent) } ?? SchemeSlot.forTab(session.agentType)
                        return TerminalSchemes.resolve(profileSchemeId: profile?.terminalSchemeId, slot: slot)
                    },
                    presets: agentProfiles.profiles.map { p in
                        (id: p.id, title: p.name,
                         detail: [p.workingDir, p.gitBranch.isEmpty ? "" : "⎇ " + p.gitBranch]
                            .filter { !$0.isEmpty }.joined(separator: " "))
                    },
                    onOpen: { id in
                        tabsManager.selectedSessionId = id
                        showHome = false
                        ensureSessionStarted(for: id)
                    },
                    onAdd: { handleAddNewSession() },
                    onLaunchPreset: { id in
                        guard canOpenNewSession() else { return }
                        guard let profile = agentProfiles.profiles.first(where: { $0.id == id }),
                              case .success(let cmd) = profile.launchCommand() else { return }
                        let s = tabsManager.addAgentSession(
                            profile: profile,
                            command: cmd,
                            serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                        )
                        ensureSessionStarted(for: s.id)
                    },
                    onRename: { id, name in tabsManager.renameSession(id: id, newTitle: name) },
                    onClose: { id in
                        let session = tabsManager.sessions.first(where: { $0.id == id })
                        let server = serversManager.profile(for: session?.serverId) ?? serversManager.activeProfile
                        let privateKey = KeychainManager.shared.getPrivateKey(id: server?.keyId)
                        tabsManager.closeSession(id: id, server: server, privateKey: privateKey)
                    },
                    isChatMode: { tabsManager.viewModel(for: $0.id)?.isChatMode == true },
                    onSessionAction: { runSessionAction($0, $1) }
                )
                }
            } else {
            VStack(spacing: 0) {
                // Top Multi-Session Tab Switcher (Compact)
                SessionTabsView(
                    sessions: tabsManager.sessions,
                    selectedSessionId: $tabsManager.selectedSessionId,
                    sessionStates: tabsManager.sessionStates,
                    serverNameProvider: { session in
                        serversManager.profile(for: session.serverId)?.name ?? serversManager.activeProfile?.name
                    },
                    iconProvider: { session in
                        let profile = agentProfiles.profiles.first { $0.id == session.profileId }
                        return (ProfileIcons.symbol(profile, tabType: session.agentType), ProfileIcons.color(profile))
                    },
                    onSelect: { id in
                        tabsManager.selectedSessionId = id
                        ensureSessionStarted(for: id)
                    },
                    onAddSession: {
                        handleAddNewSession()
                    },
                    presets: agentProfiles.profiles.map { p in
                        (id: p.id, title: p.name,
                         detail: [p.workingDir, p.gitBranch.isEmpty ? "" : "⎇ " + p.gitBranch]
                            .filter { !$0.isEmpty }.joined(separator: " "))
                    },
                    onLaunchPreset: { id in
                        guard canOpenNewSession() else { return }
                        guard let profile = agentProfiles.profiles.first(where: { $0.id == id }),
                              case .success(let cmd) = profile.launchCommand() else { return }
                        let s = tabsManager.addAgentSession(
                            profile: profile,
                            command: cmd,
                            serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                        )
                        ensureSessionStarted(for: s.id)
                    },
                    onRename: { id, newTitle in
                        tabsManager.renameSession(id: id, newTitle: newTitle)
                    },
                    onClose: { id in
                        let session = tabsManager.sessions.first(where: { $0.id == id })
                        let server = serversManager.profile(for: session?.serverId) ?? serversManager.activeProfile
                        let privateKey = KeychainManager.shared.getPrivateKey(id: server?.keyId)
                        tabsManager.closeSession(id: id, server: server, privateKey: privateKey)
                    },
                    isChatMode: { tabsManager.viewModel(for: $0.id)?.isChatMode == true },
                    onSessionAction: { runSessionAction($0, $1) }
                )
                .frame(height: 32)


                Divider()

                if usageStripEnabled {
                    SessionUsageStrip(monitor: usageMonitor) { refreshUsage(force: true) }
                }

                approvalViews

                // Main Terminal Rendering Area
                if let activeVM = tabsManager.activeViewModel {
                    ActiveTerminalSessionView(
                        viewModel: activeVM,
                        isCtrlLocked: $isCtrlLocked,
                        scheme: activeScheme,
                        onRetry: {
                            ensureSessionStarted(for: tabsManager.selectedSessionId, force: true)
                        },
                        onOpenAiAssistant: {
                            presentSheet(.aiAssistant)
                        },
                        onOpenSnippets: {
                            presentSheet(.snippets)
                        }
                    )
                    .id(activeVM.session.id)
                } else {
                    ProgressView("Initializing session...")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            }
            }
            .overlay {
                if scenePhase == .background {
                    ZStack {
                        Color.black
                        VStack(spacing: 12) {
                            Image(systemName: "lock.shield.fill")
                                .font(.system(size: 44))
                                .foregroundColor(.appAccent)
                            Text("Session Suspended")
                                .font(.headline)
                                .foregroundColor(.white)
                            Text("Zero Battery Drain Active")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    .ignoresSafeArea()
                }
            }
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if showHome {
                    ToolbarItem(placement: .topBarLeading) { assistantButton }
                    ToolbarItem(placement: .principal) {
                        Text("Sessions").font(.system(size: 15, weight: .bold))
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button { presentSheet(.settings) } label: {
                            Image(systemName: "gearshape").font(.system(size: 17))
                        }
                        .accessibilityLabel("Settings")
                    }
                } else {
                ToolbarItem(placement: .topBarLeading) {
                    Button { dismissKeyboard(); showHome = true } label: {
                        Image(systemName: "square.grid.2x2").font(.system(size: 17))
                    }
                    .contextMenu { jumpMenu }
                    .accessibilityLabel("All sessions")
                    .accessibilityHint("Tap for the list of sessions. Touch and hold to switch session, pick a server or start a new session.")
                }
                ToolbarItem(placement: .principal) {
                    HStack(spacing: 4) {
                        Image(systemName: "terminal")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(.appAccent)
                        Text(tabsManager.activeSession?.title ?? "Terminal")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundColor(.primary)
                        if let server = serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile {
                            Text("• \(server.name)")
                                .font(.system(size: 11, weight: .regular))
                                .foregroundColor(.secondary)
                        }
                    }
                }
                // One deliberate button; every other action lives in the panel it opens, so iOS
                // never has to build its own (unlabeled) overflow menu.
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 14) {
                    Button {
                        dismissKeyboard()
                        showActionPanel = true
                    } label: {
                        Image(systemName: "ellipsis.circle")
                            .font(.system(size: 17))
                    }
                    .accessibilityLabel("More actions")
                    .accessibilityHint("Opens the quick menu: AI assistant, snippets, commands, files, host monitor and more")
                    .popover(isPresented: $showActionPanel, arrowEdge: .top) {
                        ActionPanelView(
                            isChatMode: tabsManager.activeViewModel?.isChatMode == true,
                            keyVisible: false,
                            isConnected: tabsManager.activeViewModel?.state == .connected,
                            onSelect: { performAction($0) }
                        )
                        .presentationCompactAdaptation(.popover)
                    }
                    }
                }
                }
            }
            .sheet(isPresented: $showPaywall) {
                PaywallView(reason: "The free version keeps up to \(FreeTier.maxSessions) open sessions. Pro removes the limit.")
            }
            .fullScreenCover(isPresented: $showAssistant) {
                if let activeVM = assistantVM {
                    AiAssistantSheetView(viewModel: activeVM, serverName: assistantServerName) {
                        showAssistant = false
                    }
                }
            }
            .sheet(item: $activeSheet) { sheet in
                switch sheet {
                case .settings:
                    SettingsView(
                        serversManager: serversManager,
                        agentsPage: tabsManager.activeViewModel.map { vm in
                            AnyView(
                                AgentProfilesSheetView(
                                    manager: agentProfiles,
                                    viewModel: vm,
                                    onLaunch: { profile, command in
                                        activeSheet = nil
                                        guard canOpenNewSession() else { return }
                                        let s = tabsManager.addAgentSession(
                                            profile: profile,
                                            command: command,
                                            serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                                        )
                                        ensureSessionStarted(for: s.id)
                                    },
                                    embedded: true
                                )
                            )
                        }
                    )
                        .onDisappear {
                            ensureSessionStarted(for: tabsManager.selectedSessionId)
                        }
                case .publicKey:
                    PublicKeySheetView(
                        serverName: (serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile)?.name,
                        activeKeyId: activeKeyId,
                        onClose: { activeSheet = nil }
                    )
                case .tmuxSessions:
                    TmuxSessionsSheetView(
                        serversManager: serversManager,
                        initialServerId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId,
                        configDirs: agentProfiles.profiles.filter { $0.agent == .claudeCode }.compactMap { $0.configDir }
                    ) { sessionName, server in
                        let alreadyOpen = tabsManager.sessions.contains {
                            $0.sessionName == sessionName && $0.serverId == server.id
                        }
                        guard alreadyOpen || canOpenNewSession() else { return }
                        let newSession = tabsManager.openOrSwitchToSession(sessionName: sessionName, serverId: server.id)
                        ensureSessionStarted(for: newSession.id)
                    }
                case .diffViewer:
                    if let activeVM = tabsManager.activeViewModel {
                        GitLogSheetView(viewModel: activeVM)
                    }
                case .webPreview:
                    if let activeVM = tabsManager.activeViewModel {
                        WebPreviewSheetView(viewModel: activeVM)
                    }
                case .aiAssistant:
                    EmptyView() // shown full screen, see `showAssistant`
                case .commands:
                    if let activeVM = tabsManager.activeViewModel {
                        CommandPaletteSheetView(
                            viewModel: activeVM,
                            tabAgent: tabsManager.activeSession?.agentType ?? .customShell,
                            onClose: { activeSheet = nil }
                        )
                    }
                case .sessionPermissions:
                    if let activeVM = tabsManager.activeViewModel, let tab = tabsManager.activeSession {
                        SessionPermissionsSheetView(
                            tab: tab,
                            profile: agentProfiles.profiles.first(where: { $0.id == tab.profileId }),
                            handle: activeVM.state == .connected ? activeVM.sessionHandle : nil,
                            onClose: { activeSheet = nil }
                        )
                    }
                case .snippets:
                    if let activeVM = tabsManager.activeViewModel {
                        SnippetsSheetView(viewModel: activeVM) {
                            activeSheet = nil
                        }
                    }
                case .telemetry:
                    if let activeVM = tabsManager.activeViewModel {
                        HostTelemetrySheetView(viewModel: activeVM) {
                            activeSheet = nil
                        }
                    }
                case .agentProfiles:
                    if let activeVM = tabsManager.activeViewModel {
                        AgentProfilesSheetView(
                            manager: agentProfiles,
                            viewModel: activeVM,
                            onLaunch: { profile, command in
                                activeSheet = nil
                                guard canOpenNewSession() else { return }
                                let s = tabsManager.addAgentSession(
                                    profile: profile,
                                    command: command,
                                    serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                                )
                                ensureSessionStarted(for: s.id)
                            },
                            onClose: { activeSheet = nil }
                        )
                    }
                case .fileBrowser:
                    if let activeVM = tabsManager.activeViewModel {
                        FileBrowserSheetView(viewModel: activeVM) {
                            activeSheet = nil
                        }
                    }
                }
            }
            .overlay {
                if showServerPicker {
                    ServerPickerPopup(
                        title: serverPickerContext == .startup ? "Open terminal on…" : "Open new tab on…",
                        subtitle: serverPickerContext == .startup
                            ? "More than one server is online. Pick the one to connect to."
                            : "More than one server is online. Pick where this tab should run.",
                        servers: pingingServers,
                        statuses: serverStatuses,
                        currentId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId,
                        onSelect: { server in
                            withAnimation(.easeOut(duration: 0.15)) { showServerPicker = false }
                            handleServerChosen(server)
                        },
                        onCancel: {
                            withAnimation(.easeOut(duration: 0.15)) { showServerPicker = false }
                        }
                    )
                }
            }
            .animation(.easeOut(duration: 0.15), value: showServerPicker)
            .onAppear {
                ensureSessionStarted(for: tabsManager.selectedSessionId)
                checkStartupServersIfMultiple()
                startApprovalPolling()
            }
            .onChange(of: tabsManager.sessions.count) { old, new in
                // A session was just started (new tab, agent profile, tmux sheet): show it.
                if new > old, showHome { showHome = false }
            }
            .onChange(of: scenePhase) { _, newPhase in
                handleScenePhaseChange(newPhase)
            }
            // Model usage line: first read shortly after a tab is shown (the host session is
            // starting), then every 90 s while the app is in the foreground.
            .task(id: tabsManager.selectedSessionId) {
                try? await Task.sleep(nanoseconds: 3_000_000_000)
                while !Task.isCancelled {
                    refreshUsage()
                    try? await Task.sleep(nanoseconds: 90_000_000_000)
                }
            }
            .onChange(of: tabsManager.activeViewModel?.state) { _, state in
                if state == .connected { refreshUsage() }
            }
        }
    }

    /// Runs a panel action. The popover is dismissed first; sheets are presented after it has
    /// finished closing (presenting while a popover is still dismissing is dropped by UIKit).
    private func performAction(_ action: AppAction) {
        showActionPanel = false
        if action == .chatToggle {
            withAnimation(.easeInOut(duration: 0.2)) {
                tabsManager.activeViewModel?.toggleChatMode()
            }
            return
        }
        let sheet: ActiveModalSheet
        switch action {
        case .tmuxSessions: sheet = .tmuxSessions
        case .aiAssistant: sheet = .aiAssistant
        case .snippets: sheet = .snippets
        case .commands: sheet = .commands
        case .sessionPermissions: sheet = .sessionPermissions
        case .agentProfiles: sheet = .agentProfiles
        case .fileBrowser: sheet = .fileBrowser
        case .telemetry: sheet = .telemetry
        case .webPreview: sheet = .webPreview
        case .diffViewer: sheet = .diffViewer
        case .settings: sheet = .settings
        case .publicKey: sheet = .publicKey
        case .chatToggle: return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            presentSheet(sheet)
        }
    }

    private func handleServerChosen(_ server: ServerProfile) {
        switch serverPickerContext {
        case .startup:
            print("[MainView] User selected server '\(server.name)' for initial session")
            tabsManager.setSessionServer(id: tabsManager.selectedSessionId, serverId: server.id)
            ensureSessionStarted(for: tabsManager.selectedSessionId, force: true)
        case .newTab:
            print("[MainView] User selected server '\(server.name)' for new tab")
            let newSession = addNewTab(serverId: server.id)
            ensureSessionStarted(for: newSession.id)
        }
    }

    private func checkStartupServersIfMultiple() {
        let profiles = serversManager.profiles
        guard profiles.count >= 2 else { return }

        // If sessions are already restored/open, or the active session already has an assigned server,
        // do not prompt the user on startup.
        if tabsManager.sessions.count > 1 {
            print("[MainView] \(tabsManager.sessions.count) sessions already open. Skipping startup server picker.")
            return
        }
        if let active = tabsManager.activeSession, active.serverId != nil {
            print("[MainView] Active session already has assigned server. Skipping startup server picker.")
            return
        }

        Task {
            let pinging = await ServerPingService.checkReachableServers(profiles: profiles)
            await MainActor.run {
                guard self.tabsManager.sessions.count <= 1 else { return }
                guard self.tabsManager.activeSession?.serverId == nil else { return }
                if pinging.count >= 2 {
                    print("[MainView] \(pinging.count) servers pinging on startup. Presenting choice.")
                    self.pingingServers = pinging
                    self.serverStatuses = Dictionary(uniqueKeysWithValues: pinging.map { ($0.id, ServerReachability.online) })
                    self.serverPickerContext = .startup
                    self.showServerPicker = true
                }
            }
        }
    }

    /// Opens a new tab, auto-starting the configured agent profile (plain shell if none/invalid).
    private func addNewTab(serverId: UUID?) -> TabSession {
        if let profile = agentProfiles.autoStartProfile, case .success(let cmd) = profile.launchCommand() {
            return tabsManager.addAgentSession(profile: profile, command: cmd, serverId: serverId)
        }
        return tabsManager.addSession(serverId: serverId)
    }

    /// Key of the server behind the visible tab (`nil` = the default key).
    private var activeKeyId: String? {
        (serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile)?.keyId
    }

    private func openSession(_ id: UUID) {
        tabsManager.selectedSessionId = id
        ensureSessionStarted(for: id)
    }

    private func startNewSession(on serverId: UUID?) {
        guard canOpenNewSession() else { return }
        let s = addNewTab(serverId: serverId)
        ensureSessionStarted(for: s.id)
    }

    private func launchPreset(_ id: String) {
        guard canOpenNewSession() else { return }
        guard let profile = agentProfiles.profiles.first(where: { $0.id == id }),
              case .success(let cmd) = profile.launchCommand() else { return }
        let s = tabsManager.addAgentSession(
            profile: profile,
            command: cmd,
            serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
        )
        ensureSessionStarted(for: s.id)
    }

    /// Long press on the "all sessions" button: jump to another open session, look at the servers
    /// and their sessions, or start a new one (plain, from a preset, or with new settings).
    @ViewBuilder
    private var jumpMenu: some View {
        let others = tabsManager.sessions.filter { $0.id != tabsManager.selectedSessionId }
        if !others.isEmpty {
            Section("Other sessions") {
                ForEach(others) { session in
                    Button { openSession(session.id) } label: {
                        let profile = agentProfiles.profiles.first { $0.id == session.profileId }
                        Label(session.title, systemImage: ProfileIcons.symbol(profile, tabType: session.agentType))
                    }
                }
            }
        }
        let servers = serversManager.profiles
        if !servers.isEmpty {
            Section("Servers") {
                ForEach(servers) { server in
                    let onServer = tabsManager.sessions.filter {
                        ($0.serverId ?? serversManager.activeProfileId) == server.id
                    }
                    Menu {
                        ForEach(onServer) { session in
                            Button { openSession(session.id) } label: {
                                Label(session.title, systemImage: session.id == tabsManager.selectedSessionId ? "checkmark" : "terminal")
                            }
                        }
                        Button { startNewSession(on: server.id) } label: {
                            Label("New session here", systemImage: "plus")
                        }
                    } label: {
                        Label(onServer.isEmpty ? server.name : "\(server.name) (\(onServer.count))", systemImage: "server.rack")
                    }
                }
            }
        }
        Section("New session") {
            Button { startNewSession(on: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId) } label: {
                Label("New terminal", systemImage: "plus")
            }
            if !agentProfiles.profiles.isEmpty {
                Menu {
                    ForEach(agentProfiles.profiles, id: \.id) { p in
                        Button { launchPreset(p.id) } label: {
                            Text(p.workingDir.isEmpty ? p.name : "\(p.name) — \(p.workingDir)")
                        }
                    }
                } label: {
                    Label("From a preset", systemImage: "play.rectangle.on.rectangle")
                }
            }
            Button { performAction(.agentProfiles) } label: {
                Label("With new settings…", systemImage: "slider.horizontal.3")
            }
            .disabled(tabsManager.activeViewModel?.state != .connected)
        }
    }

    private func openAssistant(through vm: TerminalSessionViewModel, serverName: String) {
        vm.aiError = nil
        assistantVM = vm
        assistantServerName = serverName
        showAssistant = true
    }

    private func sessions(on server: ServerProfile) -> [TabSession] {
        tabsManager.sessions.filter {
            (serversManager.profile(for: $0.serverId) ?? serversManager.activeProfile)?.id == server.id
        }
    }

    /// AI assistant button on the sessions screen. The assistant talks to a server's model through
    /// one of its sessions, so the menu asks only what is ambiguous: the server when there are
    /// several, then the session when that server has more than one open.
    @ViewBuilder
    private var assistantButton: some View {
        let servers = serversManager.profiles
        let icon = Image(systemName: "wand.and.stars").font(.system(size: 17))
        if servers.count >= 2 {
            Menu {
                Section("Model on server") {
                    ForEach(servers) { server in assistantChoice(for: server) }
                }
            } label: { icon }
            .accessibilityLabel("AI assistant")
            .accessibilityHint("Choose the server whose model answers")
        } else if let server = servers.first ?? serversManager.activeProfile, sessions(on: server).count > 1 {
            Menu {
                Section("Run commands in") { sessionButtons(on: server) }
            } label: { icon }
            .accessibilityLabel("AI assistant")
            .accessibilityHint("Choose the session the assistant works through")
        } else {
            Button { openAssistant(on: servers.first ?? serversManager.activeProfile) } label: { icon }
                .accessibilityLabel("AI assistant")
        }
    }

    @ViewBuilder
    private func assistantChoice(for server: ServerProfile) -> some View {
        if sessions(on: server).count > 1 {
            Menu {
                sessionButtons(on: server)
            } label: { Label(server.name, systemImage: "server.rack") }
        } else {
            Button { openAssistant(on: server) } label: { Label(server.name, systemImage: "server.rack") }
        }
    }

    @ViewBuilder
    private func sessionButtons(on server: ServerProfile) -> some View {
        ForEach(sessions(on: server)) { session in
            Button {
                guard let vm = tabsManager.viewModel(for: session.id) else { return }
                if tabsManager.sessionStates[session.id] != .connected { ensureSessionStarted(for: session.id) }
                openAssistant(through: vm, serverName: server.name)
            } label: { Text(session.title) }
        }
    }

    /// The assistant reaches a server's model through a session on that server: a connected one
    /// if there is any, else one that gets reconnected, else a new terminal is started there.
    private func openAssistant(on server: ServerProfile?) {
        guard let server else { return }
        let onServer = sessions(on: server)
        if let live = onServer.first(where: { tabsManager.sessionStates[$0.id] == .connected }),
           let vm = tabsManager.viewModel(for: live.id) {
            openAssistant(through: vm, serverName: server.name)
            return
        }
        if let any = onServer.first, let vm = tabsManager.viewModel(for: any.id) {
            ensureSessionStarted(for: any.id)
            openAssistant(through: vm, serverName: server.name)
            return
        }
        guard canOpenNewSession() else { return }
        let session = addNewTab(serverId: server.id)
        ensureSessionStarted(for: session.id)
        if let vm = tabsManager.viewModel(for: session.id) {
            openAssistant(through: vm, serverName: server.name)
        }
    }

    /// A session-menu action. It belongs to the pressed tab or card, not necessarily the selected
    /// one, so that session is opened first (leaving the main screen).
    private func runSessionAction(_ id: UUID, _ action: AppAction) {
        if id != tabsManager.selectedSessionId {
            tabsManager.selectedSessionId = id
            ensureSessionStarted(for: id)
        }
        showHome = false
        performAction(action)
    }

    /// Free tier: a limited number of open tabs. Past it the paywall opens instead.
    private func canOpenNewSession() -> Bool {
        if FreeTier.canAddSession(open: tabsManager.sessions.count, isPro: subscriptions.unlocked) { return true }
        // A sheet or popover may still be closing; presenting during that is dropped by UIKit.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { showPaywall = true }
        return false
    }

    private func handleAddNewSession() {
        guard canOpenNewSession() else { return }
        let profiles = serversManager.profiles
        guard profiles.count >= 2 else {
            let newSession = addNewTab(serverId: serversManager.activeProfileId)
            ensureSessionStarted(for: newSession.id)
            return
        }

        Task {
            let pinging = await ServerPingService.checkReachableServers(profiles: profiles)
            await MainActor.run {
                if pinging.count >= 2 {
                    self.pingingServers = pinging
                    self.serverStatuses = Dictionary(uniqueKeysWithValues: pinging.map { ($0.id, ServerReachability.online) })
                    self.serverPickerContext = .newTab
                    self.showServerPicker = true
                } else if let onlyOne = pinging.first {
                    let newSession = self.addNewTab(serverId: onlyOne.id)
                    self.ensureSessionStarted(for: newSession.id)
                } else {
                    let newSession = self.addNewTab(serverId: self.serversManager.activeProfileId)
                    self.ensureSessionStarted(for: newSession.id)
                }
            }
        }
    }

    private func switchSessionServer(for sessionId: UUID, to server: ServerProfile) {
        tabsManager.setSessionServer(id: sessionId, serverId: server.id)
        ensureSessionStarted(for: sessionId, force: true)
    }

    private func ensureSessionStarted(for id: UUID, force: Bool = false) {
        guard let vm = tabsManager.viewModel(for: id) else { return }
        let session = vm.session
        let server = serversManager.profile(for: session.serverId) ?? serversManager.activeProfile
        guard let s = server, !s.host.isEmpty, !s.username.isEmpty else {
            print("[MainView] No valid server profile found. Showing Settings.")
            if activeSheet == nil {
                activeSheet = .settings
            }
            return
        }
        guard let privateKey = KeychainManager.shared.getPrivateKey(id: s.keyId), !privateKey.isEmpty else {
            print("[MainView] No private key found in Keychain. Showing Settings.")
            if activeSheet == nil {
                activeSheet = .settings
            }
            return
        }

        vm.pinnedHostKey = s.hostKeyFingerprint
        let serverId = s.id
        vm.onHostKeyObserved = { fingerprint in
            serversManager.pinHostKey(serverId: serverId, fingerprint: fingerprint)
        }

        print("[MainView] ensureSessionStarted(for: \(id), vmId=\(vm.instanceId), force: \(force)) server='\(s.name)' (\(s.username)@\(s.host):\(s.port))")

        if force {
            vm.forceRestart(
                host: s.host,
                port: s.port,
                username: s.username,
                privateKeyPEM: privateKey
            )
        } else {
            vm.start(
                host: s.host,
                port: s.port,
                username: s.username,
                privateKeyPEM: privateKey
            )
        }
    }

    // MARK: - Zero Battery Drain Lifecycle Handling

    private func startApprovalPolling() {
        WatchBridge.shared.attach(to: approvals)
        approvals.start {
            tabsManager.sessions.compactMap { session in
                guard let vm = tabsManager.viewModel(for: session.id) else { return nil }
                let server = serversManager.profile(for: session.serverId) ?? serversManager.activeProfile
                return ApprovalPollTarget(
                    serverKey: server?.id.uuidString ?? "default",
                    serverName: server?.name ?? "Host",
                    viewModel: vm
                )
            }
        }
    }

    private func handleScenePhaseChange(_ phase: ScenePhase) {
        switch phase {
        case .background:
            // Zero Battery Drain: gracefully close all TCP sockets immediately
            // tmux maintains long-running agent tasks on the MacBook.
            approvals.stop()
            tabsManager.disconnectAllForBackground()
        case .active:
            // Fast resume: instantaneously reconnect socket and re-attach to active tmux session
            tabsManager.reconnectActiveOnForeground()
            startApprovalPolling()
        case .inactive:
            break
        @unknown default:
            break
        }
    }
}

struct ActiveTerminalSessionView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Binding var isCtrlLocked: Bool
    let scheme: TerminalScheme
    let onRetry: () -> Void
    let onOpenAiAssistant: () -> Void
    let onOpenSnippets: () -> Void
    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false

    var body: some View {
        VStack(spacing: 0) {
            ZStack(alignment: .top) {
                if viewModel.isChatMode {
                    StructuredTranscriptView(viewModel: viewModel)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    #if canImport(UIKit)
                    TerminalContainerView(
                        viewModel: viewModel,
                        isCtrlLocked: $isCtrlLocked,
                        scheme: scheme
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    #else
                    Text("SwiftTerm rendering is native on iOS")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    #endif
                }

                // Top Connection Status Banner
                switch viewModel.state {
                case .connecting, .reconnecting:
                    HStack(spacing: 8) {
                        ProgressView()
                            .progressViewStyle(CircularProgressViewStyle(tint: .white))
                            .scaleEffect(0.8)
                        Text(viewModel.state == .reconnecting ? "Reconnecting to tmux..." : "Connecting to host...")
                            .font(.system(size: 13, weight: .medium))
                            .foregroundColor(.white)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.appAccent.opacity(0.85))
                    .cornerRadius(20)
                    .padding(.top, 8)
                    .shadow(radius: 4)

                case .failed(let reason):
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.yellow)
                        Text(reason)
                            .font(.system(size: 12))
                            .lineLimit(2)
                            .foregroundColor(.white)
                        Button(action: onRetry) {
                            Text("Retry")
                                .font(.system(size: 12, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.white.opacity(0.25))
                                .cornerRadius(6)
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.red.opacity(0.9))
                    .cornerRadius(12)
                    .padding(.top, 8)
                    .padding(.horizontal, 16)
                    .shadow(radius: 4)

                case .disconnected:
                    HStack(spacing: 8) {
                        Image(systemName: "bolt.slash.fill")
                            .foregroundColor(.secondary)
                        Text("Disconnected")
                            .font(.system(size: 12))
                            .foregroundColor(.white)
                        Button(action: onRetry) {
                            Text("Connect")
                                .font(.system(size: 12, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.appAccent)
                                .cornerRadius(6)
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Color(white: 0.15).opacity(0.9))
                    .cornerRadius(20)
                    .padding(.top, 8)

                case .connected:
                    EmptyView()
                }
            }

            #if canImport(UIKit)
            if !viewModel.isChatMode {
                KeyboardAccessoryView(
                    onSendBytes: { bytes in
                        viewModel.sendInput(Data(bytes))
                    },
                    onAiAssistantTap: AiAssistantVisibility
                        .resolved(raw: aiVisibilityRaw, legacyAlways: legacyAlwaysShowAi)
                        .isVisible(connected: true) ? { onOpenAiAssistant() } : nil,
                    onSnippetsTap: {
                        onOpenSnippets()
                    }
                )
            }
            #endif
        }
    }
}


/// The public key to add to `~/.ssh/authorized_keys`: the active server's key first, then the others.
struct PublicKeySheetView: View {
    let serverName: String?
    let activeKeyId: String?
    let onClose: () -> Void
    @ObservedObject private var keys = SSHKeyStore.shared
    @State private var copied: String? = nil

    private var rows: [(id: String?, name: String)] {
        let all: [(id: String?, name: String)] =
            [(nil, "Default key")] + keys.extra.map { (Optional($0.id), $0.name) }
        return all.sorted { ($0.id == activeKeyId ? 0 : 1) < ($1.id == activeKeyId ? 0 : 1) }
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(rows, id: \.id) { row in
                    let pub = KeychainManager.shared.getOrDerivePublicKey(id: row.id)
                    Section(header: Text(row.id == activeKeyId ? "\(row.name) — used by \(serverName ?? "this server")" : row.name)) {
                        if let pub, !pub.isEmpty {
                            Text(pub).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                            Button(copied == (row.id ?? "default") ? "Copied" : "Copy public key") {
                                KeychainManager.shared.copyPublicKeyToClipboard(pub)
                                copied = row.id ?? "default"
                            }
                        } else {
                            Text(row.id == nil ? "No default key yet: create one in Settings." : "Key is missing from the Keychain.")
                                .font(.caption).foregroundColor(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("Public key")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
        }
    }
}
