import SwiftUI
import SshCoreBridge

/// The jump menu and the routing of the AI assistant (which server and session it goes through).
extension MainTerminalView {
    @ViewBuilder
    var jumpMenu: some View {
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

    /// Every open session, so commands from the assistant can go to any of them.
    var assistantTargets: [AssistantTarget] {
        tabsManager.sessions.compactMap { session in
            guard let vm = tabsManager.viewModel(for: session.id) else { return nil }
            let server = serversManager.profile(for: session.serverId) ?? serversManager.activeProfile
            return AssistantTarget(id: session.id, title: session.title, serverName: server?.name ?? "", viewModel: vm)
        }
    }

    func openAssistant(through vm: TerminalSessionViewModel, server: ServerProfile) {
        let assistant = assistants.session(for: server.id)
        assistant.aiError = nil
        assistantRoute = AssistantRoute(viewModel: vm, assistant: assistant, serverName: server.name)
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
    var assistantButton: some View {
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
                openAssistant(through: vm, server: server)
            } label: { Text(session.title) }
        }
    }

    /// The assistant reaches a server's model through a session on that server: a connected one
    /// if there is any, else one that gets reconnected, else a new terminal is started there.
    func openAssistant(on server: ServerProfile?) {
        guard let server else { return }
        let onServer = sessions(on: server)
        if let live = onServer.first(where: { tabsManager.sessionStates[$0.id] == .connected }),
           let vm = tabsManager.viewModel(for: live.id) {
            openAssistant(through: vm, server: server)
            return
        }
        if let any = onServer.first, let vm = tabsManager.viewModel(for: any.id) {
            ensureSessionStarted(for: any.id)
            openAssistant(through: vm, server: server)
            return
        }
        guard canOpenNewSession() else { return }
        let session = addNewTab(serverId: server.id)
        ensureSessionStarted(for: session.id)
        if let vm = tabsManager.viewModel(for: session.id) {
            openAssistant(through: vm, server: server)
        }
    }

    /// A session-menu action. It belongs to the pressed tab or card, not necessarily the selected
    /// one, so that session is opened first (leaving the main screen).
}
