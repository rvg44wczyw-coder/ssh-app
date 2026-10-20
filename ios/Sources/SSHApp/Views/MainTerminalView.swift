import SwiftUI
import UserNotifications
import SshCoreBridge

struct PendingLocalLaunch {
    let profile: SavedAgentProfile
    let command: String
}

/// Where the AI assistant screen goes: the session whose connection reaches the host model.
struct AssistantRoute: Identifiable, Hashable {
    let id = UUID()
    let viewModel: TerminalSessionViewModel
    let assistant: AssistantSession
    let serverName: String
    static func == (a: AssistantRoute, b: AssistantRoute) -> Bool { a.id == b.id }
    func hash(into h: inout Hasher) { h.combine(id) }
}

public struct MainTerminalView: View {
    @Environment(\.scenePhase) private var scenePhase

    @StateObject var serversManager = ServerProfilesManager()
    @StateObject var tabsManager = TerminalTabsManager()
    @StateObject var agentProfiles = AgentProfilesManager()

    private enum ServerPickerContext {
        case startup
        case newTab
    }

    enum ActiveModalSheet: String, Identifiable {
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
        return TerminalSchemes.resolve(tabSchemeId: tab.schemeId, profileSchemeId: profile?.terminalSchemeId, slot: slot)
    }
    @State var activeSheet: ActiveModalSheet? = nil
    /// The AI assistant is a screen of its own (pushed on the navigation stack), not a card over
    /// the terminal. One value carries both "open" and "through which session", so the screen can
    /// never appear without its session.
    @State var assistantRoute: AssistantRoute?
    @State private var pendingLocal: PendingLocalLaunch?
    @StateObject private var approvals = ApprovalsCenter()
    @ObservedObject private var push = PushManager.shared
    @State private var deliveredNotices: Set<String> = []
    /// The tab whose agent the user is about to restart (asks new / continue first).
    @State private var restartTarget: UUID?
    @State private var restartError: String?
    /// Hook notices per tab (finished / needs you), cleared when the tab is opened.
    @State private var hookAttention: [UUID: SessionAttention] = [:]
    /// Bumped by the watcher so the symbols are read again (the view models are not observed here).
    @State private var attentionTick = 0
    /// Chat and command state of the AI assistant, one per server (outlives tabs and the screen).
    @StateObject var assistants = AssistantStore()

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

    /// What the tab's agent wants from you: a pending approval (the card), a question on its screen
    /// (the watcher below), or a hook notice (finished / needs you) until you open the tab.
    private func attention(for tab: TabSession) -> SessionAttention? {
        _ = attentionTick
        var found: [SessionAttention] = []
        if let a = hookAttention[tab.id] { found.append(a) }
        if approvals.entries.contains(where: { $0.isApproval && approvals.tmuxName(for: $0) == tab.sessionName }) {
            found.append(.approval)
        }
        let viewing = !showHome && tab.id == tabsManager.selectedSessionId
        if let vm = tabsManager.viewModel(for: tab.id), vm.screenPrompt != nil, !(viewing && vm.isChatMode) {
            found.append(.question)
        }
        return found.max()
    }

    /// Every few seconds, while the app is in front: ask each connected tab's pane whether a question
    /// is on it. One cheap checksum call per tab (nothing is parsed while the screen is unchanged).
    private func watchPrompts() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(6))
            guard scenePhase == .active else { continue }
            for tab in tabsManager.sessions {
                guard let vm = tabsManager.viewModel(for: tab.id) else { continue }
                if !showHome, tab.id == tabsManager.selectedSessionId, vm.isChatMode { continue }
                await vm.watchPromptOnce()
            }
            attentionTick += 1
        }
    }

    /// Notices of the hooks (an agent finished, needs you) become a symbol on the tab's card and
    /// tab until you open it; the queue entry is acknowledged. With the app closed the relay's push
    /// says the same.
    private func deliverNotices() {
        for notice in approvals.notices where !deliveredNotices.contains(notice.id) {
            deliveredNotices.insert(notice.id)
            if let tmux = approvals.tmuxName(for: notice),
               let tab = tabsManager.sessions.first(where: { $0.sessionName == tmux }),
               !(!showHome && tab.id == tabsManager.selectedSessionId) {
                let kind: SessionAttention = notice.item.event == "Stop" ? .finished : .needsYou
                hookAttention[tab.id] = max(hookAttention[tab.id] ?? kind, kind)
            }
            Task { await approvals.dismiss(notice) }
        }
    }

    /// Command that starts the tab's agent again: from its profile as it is now (so hooks and
    /// permissions set up since are picked up), else the command the tab was started with.
    private func restartCommand(for tab: TabSession, continuing: Bool) -> String? {
        if let profile = agentProfiles.profiles.first(where: { $0.id == tab.profileId }) {
            var p = profile
            if continuing, let flag = Self.continueFlag(profile.agent) { p.args.append(flag) }
            if case .success(let cmd) = p.launchCommand() { return cmd }
            return nil
        }
        return tab.command
    }

    private static func continueFlag(_ agent: AgentKind) -> String? {
        switch agent {
        case .claudeCode: return "--continue"
        case .antigravity: return "--continue"
        default: return nil
        }
    }

    private func restart(_ id: UUID, continuing: Bool) {
        restartTarget = nil
        guard let tab = tabsManager.sessions.first(where: { $0.id == id }),
              let vm = tabsManager.viewModel(for: id),
              let command = restartCommand(for: tab, continuing: continuing) else {
            restartError = "This tab has no launch settings to restart from. Start it from an agent profile."
            return
        }
        Task {
            do {
                try await vm.restartAgent(command: command)
                // Later restarts start from the profile again, without the continue flag.
                if let base = restartCommand(for: tab, continuing: false) {
                    tabsManager.setSessionCommand(id: id, command: base)
                }
            } catch {
                restartError = error.localizedDescription
            }
        }
    }

    /// A tap on a push opens the tab its `sid` names (no match: the tab is closed, nothing to open).
    private func openPushTarget() {
        guard let sid = push.pendingSid else { return }
        guard let tab = tabsManager.sessions.first(where: { push.sessionId(for: $0.sessionName) == sid }) else {
            push.pendingSid = nil
            return
        }
        push.pendingSid = nil
        openWaiting(tab.id)
    }

    private func openWaiting(_ id: UUID) {
        hookAttention[id] = nil
        tabsManager.selectedSessionId = id
        showHome = false
        if let vm = tabsManager.viewModel(for: id), !vm.isChatMode { vm.toggleChatMode() }
    }

    @ViewBuilder
    private var approvalViews: some View {
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
            configDirs: agentProfiles.claudeConfigDirs,
            force: force
        )
    }

    private func presentSheet(_ sheet: ActiveModalSheet) {
        dismissKeyboard()
        if sheet == .aiAssistant {
            guard let vm = tabsManager.activeViewModel else { return }
            guard let server = serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile else { return }
            openAssistant(through: vm, server: server)
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
                        return TerminalSchemes.resolve(tabSchemeId: session.schemeId, profileSchemeId: profile?.terminalSchemeId, slot: slot)
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
                    onLaunchPreset: { id in launchPreset(id) },
                    onRename: { id, name in tabsManager.renameSession(id: id, newTitle: name) },
                    onClose: { id in
                        let session = tabsManager.sessions.first(where: { $0.id == id })
                        let server = serversManager.profile(for: session?.serverId) ?? serversManager.activeProfile
                        let privateKey = KeychainManager.shared.getPrivateKey(id: server?.keyId)
                        tabsManager.closeSession(id: id, server: server, privateKey: privateKey)
                    },
                    isChatMode: { tabsManager.viewModel(for: $0.id)?.isChatMode == true },
                    attention: { attention(for: $0) },
                    onRestart: { restartTarget = $0 },
                    onSessionAction: { runSessionAction($0, $1) },
                    onSetScheme: { tabsManager.setSessionScheme(id: $0, schemeId: $1) },
                    assistants: assistants.withHistory(in: serversManager.profiles.map(\.id)),
                    assistantServerName: { id in serversManager.profile(for: id)?.name ?? "" },
                    onOpenAssistant: { id in openAssistant(on: serversManager.profile(for: id)) },
                    onClearAssistant: { id in assistants.session(for: id).clearAll() }
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
                    onLaunchPreset: { id in launchPreset(id) },
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
                    attention: { attention(for: $0) },
                    onRestart: { restartTarget = $0 },
                    onSessionAction: { runSessionAction($0, $1) },
                    onSetScheme: { tabsManager.setSessionScheme(id: $0, schemeId: $1) }
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
                    // Tap = all sessions, touch and hold = the jump menu. A Menu keeps its label in
                    // place while it opens; a context menu on a toolbar button let the glyph slide.
                    Menu {
                        jumpMenu
                    } label: {
                        Image(systemName: "square.grid.2x2").font(.system(size: 17))
                    } primaryAction: {
                        dismissKeyboard(); showHome = true
                    }
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
            .navigationDestination(item: $assistantRoute) { route in
                    AiAssistantSheetView(
                        viewModel: route.viewModel,
                        assistant: route.assistant,
                        serverName: route.serverName,
                        targets: assistantTargets,
                        onSent: { id in
                            tabsManager.selectedSessionId = id
                            showHome = false
                        },
                        onClose: { assistantRoute = nil }
                    )
            }
            .alert(
                "Start \(pendingLocal?.profile.name ?? "agent")?",
                isPresented: Binding(get: { pendingLocal != nil }, set: { if !$0 { pendingLocal = nil } })
            ) {
                Button("Cancel", role: .cancel) {}
                Button("Start") {
                    if let p = pendingLocal {
                        agentProfiles.acknowledge(p.profile.id)
                        startProfile(p.profile, command: p.command)
                    }
                }
            } message: {
                Text(pendingLocal.map { localLaunchMessage($0.profile) } ?? "")
            }
            .sheet(item: $activeSheet) { sheet in
                sheetContent(sheet)
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
            .onReceive(NotificationCenter.default.publisher(for: .openWorktreeTab)) { note in
                guard let title = note.userInfo?["title"] as? String,
                      let path = note.userInfo?["path"] as? String else { return }
                let serverId = tabsManager.activeSession?.serverId
                let tab = tabsManager.addWorktreeSession(title: title, path: path, serverId: serverId)
                ensureSessionStarted(for: tab.id)
            }
            .onAppear {
                ensureSessionStarted(for: tabsManager.selectedSessionId)
                checkStartupServersIfMultiple()
                startApprovalPolling()
            }
            .confirmationDialog("Restart the agent in this tab?",
                                isPresented: Binding(get: { restartTarget != nil }, set: { if !$0 { restartTarget = nil } }),
                                titleVisibility: .visible) {
                if let id = restartTarget, let tab = tabsManager.sessions.first(where: { $0.id == id }) {
                    Button("New conversation") { restart(id, continuing: false) }
                    if agentProfiles.profiles.first(where: { $0.id == tab.profileId }).map({ Self.continueFlag($0.agent) != nil }) == true {
                        Button("Continue the last conversation") { restart(id, continuing: true) }
                    }
                }
                Button("Cancel", role: .cancel) { restartTarget = nil }
            } message: {
                Text("The running agent is stopped and started again in the same tmux session with its profile's current settings, hooks and permissions included.")
            }
            .alert("Restart", isPresented: Binding(get: { restartError != nil }, set: { if !$0 { restartError = nil } })) {
                Button("OK") { restartError = nil }
            } message: { Text(restartError ?? "") }
            .task { await watchPrompts() }
            .onChange(of: approvals.notices.map(\.id)) { _, _ in deliverNotices() }
            .onChange(of: tabsManager.selectedSessionId) { _, id in hookAttention[id] = nil }
            .onChange(of: showHome) { _, home in
                if !home { hookAttention[tabsManager.selectedSessionId] = nil }
            }
            .onAppear { openPushTarget() }
            .onChange(of: push.pendingSid) { _, _ in openPushTarget() }
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
    func performAction(_ action: AppAction) {
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
    func addNewTab(serverId: UUID?) -> TabSession {
        if let profile = agentProfiles.autoStartProfile, case .success(let cmd) = profile.launchCommand(),
           !profile.isLocal || agentProfiles.isAcknowledged(profile.id) {
            return tabsManager.addAgentSession(profile: profile, command: cmd, serverId: serverId)
        }
        return tabsManager.addSession(serverId: serverId)
    }

    /// Key of the server behind the visible tab (`nil` = the default key).
    var activeKeyId: String? {
        (serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile)?.keyId
    }

    func openSession(_ id: UUID) {
        tabsManager.selectedSessionId = id
        ensureSessionStarted(for: id)
    }

    func startNewSession(on serverId: UUID?) {
        guard canOpenNewSession() else { return }
        let s = addNewTab(serverId: serverId)
        ensureSessionStarted(for: s.id)
    }

    func launchPreset(_ id: String) {
        guard let profile = agentProfiles.profiles.first(where: { $0.id == id }),
              case .success(let cmd) = profile.launchCommand() else { return }
        startProfile(profile, command: cmd)
    }

    /// Opens a tab running `profile`. A local-model agent asks for a one-time confirmation first
    /// (it may run without the phone gate, and `ollama launch` writes config on the host).
    func startProfile(_ profile: SavedAgentProfile, command: String) {
        if profile.isLocal && !agentProfiles.isAcknowledged(profile.id) {
            // A sheet or menu may still be closing; an alert presented during that is dropped.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                pendingLocal = PendingLocalLaunch(profile: profile, command: command)
            }
            return
        }
        guard canOpenNewSession() else { return }
        let s = tabsManager.addAgentSession(
            profile: profile,
            command: command,
            serverId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
        )
        ensureSessionStarted(for: s.id)
    }

    private func localLaunchMessage(_ profile: SavedAgentProfile) -> String {
        let tool = localAgentTools().first { $0.id == profile.localTool }
        let gate = tool?.hasApprovals == true
            ? "Install its approval hooks (Agent profiles → Review & install) to get phone approvals; until then it runs commands on the host without asking your phone."
            : "This tool has no approval hook: it runs commands on the host without phone approval and asks in its own terminal, if at all."
        let writes = ["opencode", "qwen", "cline"].contains(profile.localTool)
            ? " Starting it uses `ollama launch`, which writes the tool's config on the host."
            : ""
        return gate + writes
    }

    /// Long press on the "all sessions" button: jump to another open session, look at the servers
    /// and their sessions, or start a new one (plain, from a preset, or with new settings).
    private func runSessionAction(_ id: UUID, _ action: AppAction) {
        if id != tabsManager.selectedSessionId {
            tabsManager.selectedSessionId = id
            ensureSessionStarted(for: id)
        }
        showHome = false
        performAction(action)
    }

    /// Free tier: a limited number of open tabs. Past it the paywall opens instead.
    func canOpenNewSession() -> Bool {
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

    func ensureSessionStarted(for id: UUID, force: Bool = false) {
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
            assistants.interruptAll()
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
