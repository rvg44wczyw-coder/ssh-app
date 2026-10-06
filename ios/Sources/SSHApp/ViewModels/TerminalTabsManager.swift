import Foundation
import Combine
import SshCoreBridge

/// Single source of truth for all terminal tab sessions and their persistent ViewModels.
/// Held as an `@StateObject` in `MainTerminalView` so instances are never recreated.
@MainActor
public final class TerminalTabsManager: ObservableObject {
    @Published public var sessions: [TabSession] = []
    @Published public var selectedSessionId: UUID
    @Published public var sessionStates: [UUID: SessionState] = [:]

    private var viewModels: [UUID: TerminalSessionViewModel] = [:]
    private var stateCancellables: [UUID: AnyCancellable] = [:]

    private static let storageKey = "saved_tab_sessions_v1"

    public init() {
        let loaded = Self.loadSavedSessions() ?? TabSession.defaultSessions()
        self.sessions = loaded
        let initialId = loaded.first?.id ?? UUID()
        self.selectedSessionId = initialId

        for session in loaded {
            registerSession(session)
        }
    }

    private func registerSession(_ session: TabSession) {
        if viewModels[session.id] != nil { return }
        let vm = TerminalSessionViewModel(session: session)
        viewModels[session.id] = vm
        sessionStates[session.id] = vm.state

        stateCancellables[session.id] = vm.$state
            .receive(on: DispatchQueue.main)
            .sink { [weak self] newState in
                self?.sessionStates[session.id] = newState
            }
    }

    public func viewModel(for id: UUID) -> TerminalSessionViewModel? {
        if let existing = viewModels[id] {
            return existing
        }
        if let session = sessions.first(where: { $0.id == id }) {
            registerSession(session)
            return viewModels[id]
        }
        return nil
    }

    public var activeViewModel: TerminalSessionViewModel? {
        viewModel(for: selectedSessionId)
    }

    public var activeSession: TabSession? {
        sessions.first(where: { $0.id == selectedSessionId })
    }

    public func renameSession(id: UUID, newTitle: String) {
        let trimmed = newTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if let index = sessions.firstIndex(where: { $0.id == id }) {
            print("[TabsManager] Renaming session \(id) from '\(sessions[index].title)' to '\(trimmed)'")
            sessions[index].title = trimmed
            saveSessions()
        }
    }

    public func setSessionServer(id: UUID, serverId: UUID) {
        if let index = sessions.firstIndex(where: { $0.id == id }) {
            print("[TabsManager] Setting server for session \(id) to \(serverId)")
            sessions[index].serverId = serverId
            if let vm = viewModels[id] {
                vm.session.serverId = serverId
            }
            saveSessions()
        }
    }

    public func closeSession(
        id: UUID,
        server: ServerProfile? = nil,
        privateKey: String? = nil
    ) {
        guard sessions.count > 1 else {
            print("[TabsManager] Cannot close last remaining session")
            return
        }
        guard let sessionToClose = sessions.first(where: { $0.id == id }) else { return }
        let tmuxSessionName = sessionToClose.sessionName
        print("[TabsManager] Closing tab session '\(sessionToClose.title)' (tmux: \(tmuxSessionName))")

        if let vm = viewModels.removeValue(forKey: id) {
            vm.disconnectForBackground()
        }
        stateCancellables.removeValue(forKey: id)
        sessionStates.removeValue(forKey: id)

        if let index = sessions.firstIndex(where: { $0.id == id }) {
            sessions.remove(at: index)
            if selectedSessionId == id {
                let nextIndex = min(index, sessions.count - 1)
                selectedSessionId = sessions[nextIndex].id
            }
            saveSessions()
        }

        // Terminate remote tmux session if no remaining tab uses it
        let remainingUsingSameTmux = sessions.contains(where: {
            $0.sessionName == tmuxSessionName && $0.serverId == sessionToClose.serverId
        })

        if !remainingUsingSameTmux,
           let server = server,
           let privateKey = privateKey,
           !server.host.isEmpty,
           !server.username.isEmpty,
           !privateKey.isEmpty {
            let config = RemoteServerConfig(
                host: server.host,
                port: server.port,
                username: server.username,
                privateKeyPem: privateKey
            )
            Task.detached {
                print("[TabsManager] Terminating remote tmux session '\(tmuxSessionName)' on \(server.name)...")
                do {
                    try await killRemoteTmuxSession(config: config, sessionName: tmuxSessionName)
                    print("[TabsManager] Successfully terminated remote tmux session '\(tmuxSessionName)'")
                } catch {
                    print("[TabsManager] Warning: Failed to kill remote tmux session '\(tmuxSessionName)': \(error)")
                }
            }
        }
    }

    public func addSession(
        title: String? = nil,
        sessionName: String? = nil,
        serverId: UUID? = nil
    ) -> TabSession {
        let count = nextSessionNumber()
        let sessionTitle = title ?? "Terminal \(count)"
        let safeSessionName = sessionName ?? "term-\(count)"
        let newSession = TabSession(
            title: sessionTitle,
            agentType: .customShell,
            sessionName: safeSessionName,
            command: nil,
            serverId: serverId
        )
        print("[TabsManager] Adding session '\(sessionTitle)' (name=\(safeSessionName), serverId=\(String(describing: serverId)))")
        sessions.append(newSession)
        registerSession(newSession)
        selectedSessionId = newSession.id
        saveSessions()
        return newSession
    }

    public func openOrSwitchToSession(sessionName: String, serverId: UUID?) -> TabSession {
        // If an open tab already targets this tmux session and server, switch to it
        if let existing = sessions.first(where: {
            $0.sessionName == sessionName && ($0.serverId == serverId || (serverId == nil && $0.serverId == nil))
        }) {
            print("[TabsManager] Switching to existing session '\(sessionName)' (\(existing.id))")
            selectedSessionId = existing.id
            return existing
        }
        return addSession(title: sessionName, sessionName: sessionName, serverId: serverId)
    }

    private func nextSessionNumber() -> Int {
        var num = sessions.count + 1
        let existingNames = Set(sessions.map { $0.sessionName })
        while existingNames.contains("term-\(num)") {
            num += 1
        }
        return num
    }

    private func saveSessions() {
        if let data = try? JSONEncoder().encode(sessions) {
            UserDefaults.standard.set(data, forKey: Self.storageKey)
        }
    }

    private static func loadSavedSessions() -> [TabSession]? {
        guard let data = UserDefaults.standard.data(forKey: storageKey),
              let loaded = try? JSONDecoder().decode([TabSession].self, from: data),
              !loaded.isEmpty else {
            return nil
        }
        return loaded
    }

    public func disconnectAllForBackground() {
        for vm in viewModels.values {
            vm.disconnectForBackground()
        }
    }

    public func reconnectActiveOnForeground() {
        activeViewModel?.reconnectOnForeground()
    }
}
