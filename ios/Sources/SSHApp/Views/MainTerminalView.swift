import SwiftUI
import SshCoreBridge

public struct MainTerminalView: View {
    @Environment(\.scenePhase) private var scenePhase

    @StateObject private var serversManager = ServerProfilesManager()
    @StateObject private var tabsManager = TerminalTabsManager()

    private enum ServerPickerContext {
        case startup
        case newTab
    }

    @State private var isCtrlLocked: Bool = false
    @State private var showSettings: Bool = false
    @State private var showTmuxSessions: Bool = false
    @State private var showPublicKeyBanner: Bool = true
    @State private var copiedKeyNotification: Bool = false
    @State private var activePublicKey: String? = nil
    @State private var showDiffViewer: Bool = false
    @State private var showWebPreview: Bool = false
    @State private var pendingApproval: AgentApprovalRequest? = nil

    @State private var showServerPicker: Bool = false
    @State private var pingingServers: [ServerProfile] = []
    @State private var serverPickerContext: ServerPickerContext = .startup
    @State private var isCheckingServers: Bool = false

    public init() {}

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
                    onRename: { id, newTitle in
                        tabsManager.renameSession(id: id, newTitle: newTitle)
                    },
                    onClose: { id in
                        let session = tabsManager.sessions.first(where: { $0.id == id })
                        let server = serversManager.profile(for: session?.serverId) ?? serversManager.activeProfile
                        let privateKey = KeychainManager.shared.getPrivateKey()
                        tabsManager.closeSession(id: id, server: server, privateKey: privateKey)
                    }
                )
                .frame(height: 32)


                // Public Key Banner shown on App Restart
                if showPublicKeyBanner, let pubKey = activePublicKey ?? KeychainManager.shared.getOrDerivePublicKey(), !pubKey.isEmpty {
                    HStack(spacing: 6) {
                        Image(systemName: "key.fill")
                            .font(.system(size: 10))
                            .foregroundColor(.yellow)
                        Text(pubKey)
                            .font(.system(size: 9, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundColor(.primary)

                        Spacer()

                        Button(action: {
                            KeychainManager.shared.copyPublicKeyToClipboard(pubKey)
                            copiedKeyNotification = true
                            DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                                copiedKeyNotification = false
                            }
                        }) {
                            Text(copiedKeyNotification ? "Copied!" : "Copy Key")
                                .font(.system(size: 9, weight: .bold))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.accentColor.opacity(0.2))
                                .foregroundColor(.accentColor)
                                .cornerRadius(4)
                        }

                        Button(action: {
                            withAnimation(.easeInOut(duration: 0.2)) {
                                showPublicKeyBanner = false
                            }
                        }) {
                            Image(systemName: "xmark")
                                .font(.system(size: 9, weight: .bold))
                                .foregroundColor(.secondary)
                                .padding(2)
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Color.appSecondaryBackground)
                }

                Divider()

                if let approval = pendingApproval {
                    ApprovalBannerView(
                        request: approval,
                        onApprove: { req in
                            handleApprove(req)
                        },
                        onDeny: { req in
                            handleDeny(req)
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
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: { showTmuxSessions = true }) {
                        Image(systemName: "rectangle.stack")
                            .font(.system(size: 13))
                    }
                }
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
                #if os(iOS)
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button(action: { tabsManager.activeViewModel?.openAiAssistant() }) {
                        Image(systemName: "wand.and.stars")
                            .font(.system(size: 14))
                            .foregroundColor(.purple)
                    }
                    .accessibilityLabel("AI Shell Assistant")

                    Button(action: { showWebPreview = true }) {
                        Image(systemName: "globe")
                            .font(.system(size: 14))
                    }
                    .accessibilityLabel("In-App Web Preview")

                    Button(action: { showDiffViewer = true }) {
                        Image(systemName: "arrow.triangle.pull")
                            .font(.system(size: 14))
                    }
                    .accessibilityLabel("Git Diff Review")

                    Button(action: {
                        withAnimation {
                            showPublicKeyBanner.toggle()
                            if showPublicKeyBanner {
                                activePublicKey = KeychainManager.shared.getOrDerivePublicKey()
                            }
                        }
                    }) {
                        Image(systemName: "key")
                            .font(.system(size: 14))
                    }
                    .accessibilityLabel("Toggle Public Key")

                    Button(action: { showSettings = true }) {
                        Image(systemName: "gearshape")
                            .font(.system(size: 14))
                    }
                    .accessibilityLabel("Settings")
                }
                #else
                ToolbarItem(placement: .automatic) {
                    Button(action: { showSettings = true }) {
                        Image(systemName: "gearshape")
                    }
                }
                #endif
            }
            .sheet(isPresented: $showSettings, onDismiss: {
                activePublicKey = KeychainManager.shared.getOrDerivePublicKey()
                ensureSessionStarted(for: tabsManager.selectedSessionId)
            }) {
                SettingsView(serversManager: serversManager)
            }
            .sheet(isPresented: $showTmuxSessions) {
                TmuxSessionsSheetView(
                    serversManager: serversManager,
                    initialServerId: tabsManager.activeSession?.serverId ?? serversManager.activeProfileId
                ) { sessionName, server in
                    let newSession = tabsManager.openOrSwitchToSession(sessionName: sessionName, serverId: server.id)
                    ensureSessionStarted(for: newSession.id)
                }
            }
            .sheet(isPresented: $showDiffViewer) {
                let activeServer = serversManager.profile(for: tabsManager.activeSession?.serverId) ?? serversManager.activeProfile
                DiffViewerSheetView(
                    serverProfile: activeServer,
                    privateKeyOpenssh: KeychainManager.shared.getPrivateKey()
                )
            }
            .sheet(isPresented: $showWebPreview) {
                if let activeVM = tabsManager.activeViewModel {
                    WebPreviewSheetView(viewModel: activeVM)
                }
            }
            .sheet(isPresented: Binding(
                get: { tabsManager.activeViewModel?.isAiAssistantOpen ?? false },
                set: { isOpen in
                    if !isOpen { tabsManager.activeViewModel?.closeAiAssistant() }
                }
            )) {
                if let activeVM = tabsManager.activeViewModel {
                    AiAssistantSheetView(viewModel: activeVM)
                }
            }
            .confirmationDialog(
                serverPickerContext == .startup ? "Select Server to Open" : "Open New Tab on Server",
                isPresented: $showServerPicker,
                titleVisibility: .visible
            ) {
                ForEach(pingingServers) { server in
                    Button("\(server.name) (\(server.host))") {
                        handleServerChosen(server)
                    }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(serverPickerContext == .startup
                     ? "Both configured servers are online. Which server do you want to open terminal on?"
                     : "Both servers are online. Which server should this new tab connect to?")
            }
            .onAppear {
                let pubKey = KeychainManager.shared.getOrDerivePublicKey()
                activePublicKey = pubKey
                print(">>> CURRENT_PUBLIC_KEY: \(pubKey ?? "NONE") <<<")
                fflush(stdout)
                ensureSessionStarted(for: tabsManager.selectedSessionId)
                checkStartupServersIfMultiple()
            }
            .onChange(of: scenePhase) { _, newPhase in
                handleScenePhaseChange(newPhase)
            }
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
            let newSession = tabsManager.addSession(serverId: server.id)
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
                    self.serverPickerContext = .startup
                    self.showServerPicker = true
                }
            }
        }
    }

    private func handleAddNewSession() {
        let profiles = serversManager.profiles
        guard profiles.count >= 2 else {
            let newSession = tabsManager.addSession(serverId: serversManager.activeProfileId)
            ensureSessionStarted(for: newSession.id)
            return
        }

        Task {
            let pinging = await ServerPingService.checkReachableServers(profiles: profiles)
            await MainActor.run {
                if pinging.count >= 2 {
                    self.pingingServers = pinging
                    self.serverPickerContext = .newTab
                    self.showServerPicker = true
                } else if let onlyOne = pinging.first {
                    let newSession = self.tabsManager.addSession(serverId: onlyOne.id)
                    self.ensureSessionStarted(for: newSession.id)
                } else {
                    let newSession = self.tabsManager.addSession(serverId: self.serversManager.activeProfileId)
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
            showSettings = true
            return
        }
        guard let privateKey = KeychainManager.shared.getPrivateKey(), !privateKey.isEmpty else {
            print("[MainView] No private key found in Keychain. Showing Settings.")
            showSettings = true
            return
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

    private func handleScenePhaseChange(_ phase: ScenePhase) {
        switch phase {
        case .background:
            // Zero Battery Drain: gracefully close all TCP sockets immediately
            // tmux maintains long-running agent tasks on the MacBook.
            tabsManager.disconnectAllForBackground()
        case .active:
            // Fast resume: instantaneously reconnect socket and re-attach to active tmux session
            tabsManager.reconnectActiveOnForeground()
        case .inactive:
            break
        @unknown default:
            break
        }
    }

    private func handleApprove(_ req: AgentApprovalRequest) {
        if let vm = tabsManager.activeViewModel {
            vm.sendInput(Data([0x79, 0x0D]))
        }
        withAnimation {
            self.pendingApproval = nil
        }
    }

    private func handleDeny(_ req: AgentApprovalRequest) {
        if let vm = tabsManager.activeViewModel {
            vm.sendInput(Data([0x03]))
        }
        withAnimation {
            self.pendingApproval = nil
        }
    }
}

struct ActiveTerminalSessionView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Binding var isCtrlLocked: Bool
    let onRetry: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            ZStack(alignment: .top) {
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
            KeyboardAccessoryView(
                isCtrlLocked: $isCtrlLocked,
                onSendBytes: { bytes in
                    viewModel.sendInput(Data(bytes))
                },
                onAiAssistantTap: {
                    viewModel.openAiAssistant()
                }
            )
            #endif
        }
    }
}
