import SwiftUI
import SshCoreBridge

extension MainTerminalView {
    /// Content of the modal sheet picked in `activeSheet`.
    @ViewBuilder
    func sheetContent(_ sheet: ActiveModalSheet) -> some View {
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
                                startProfile(profile, command: command)
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
                configDirs: agentProfiles.claudeConfigDirs
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
            EmptyView() // pushed as a screen, see `assistantRoute`
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
                        startProfile(profile, command: command)
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
}
