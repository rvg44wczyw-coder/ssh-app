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
        case telemetry
        case fileBrowser
        case agentProfiles

        var id: String { rawValue }
    }

    @State private var isCtrlLocked: Bool = false
    @State private var activeSheet: ActiveModalSheet? = nil
    @StateObject private var approvals = ApprovalsCenter()

    @State private var showServerPicker: Bool = false
    @State private var showActionPanel: Bool = false
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

    private func presentSheet(_ sheet: ActiveModalSheet) {
        dismissKeyboard()
        if sheet == .aiAssistant {
            tabsManager.activeViewModel?.aiError = nil
        }
        activeSheet = sheet
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Top Multi-Session Tab Switcher (Compact)
                SessionTabsView(
                    sessions: tabsManager.sessions,
                    selectedSessionId: $tabsManager.selectedSessionId,
                    sessionStates: tabsManager.sessionStates,
                    serverNameProvider: { session in
                        serversManager.profile(for: session.serverId)?.name ?? serversManager.activeProfile?.name
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
                    }
                )
                .frame(height: 32)


                Divider()

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

                // Main Terminal Rendering Area
                if let activeVM = tabsManager.activeViewModel {
                    ActiveTerminalSessionView(
                        viewModel: activeVM,
                        isCtrlLocked: $isCtrlLocked,
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
            .overlay {
                if scenePhase == .background {
                    ZStack {
                        Color.black
                        VStack(spacing: 12) {
                            Image(systemName: "lock.shield.fill")
                                .font(.system(size: 44))
                                .foregroundColor(.accentColor)
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
                ToolbarItem(placement: .principal) {
                    HStack(spacing: 4) {
                        Image(systemName: "terminal")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(.accentColor)
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
                    Button {
                        dismissKeyboard()
                        showActionPanel = true
                    } label: {
                        Image(systemName: "ellipsis.circle")
                            .font(.system(size: 17))
                    }
                    .accessibilityLabel("More actions")
                    .accessibilityHint("Opens the list of actions: chat mode, tmux, AI assistant, files and more")
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
            .sheet(item: $activeSheet) { sheet in
                switch sheet {
                case .settings:
                    SettingsView(serversManager: serversManager)
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
                        initialServerId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                    ) { sessionName, server in
                        let newSession = tabsManager.openOrSwitchToSession(sessionName: sessionName, serverId: server.id)
                        ensureSessionStarted(for: newSession.id)
                    }
                case .diffViewer:
                    if let activeVM = tabsManager.activeViewModel {
                        DiffViewerSheetView(viewModel: activeVM)
                    }
                case .webPreview:
                    if let activeVM = tabsManager.activeViewModel {
                        WebPreviewSheetView(viewModel: activeVM)
                    }
                case .aiAssistant:
                    if let activeVM = tabsManager.activeViewModel {
                        AiAssistantSheetView(viewModel: activeVM) {
                            activeSheet = nil
                        }
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
            .onChange(of: scenePhase) { _, newPhase in
                handleScenePhaseChange(newPhase)
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

    private func handleAddNewSession() {
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
    let onRetry: () -> Void
    let onOpenAiAssistant: () -> Void
    let onOpenSnippets: () -> Void

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
                        isCtrlLocked: $isCtrlLocked
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
                    .background(Color.blue.opacity(0.85))
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
                                .background(Color.accentColor)
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
                    isCtrlLocked: $isCtrlLocked,
                    onSendBytes: { bytes in
                        viewModel.sendInput(Data(bytes))
                    },
                    onAiAssistantTap: {
                        onOpenAiAssistant()
                    },
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
