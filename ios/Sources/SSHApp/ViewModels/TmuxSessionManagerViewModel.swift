import Foundation
import Combine
import SshCoreBridge

@MainActor
public final class TmuxSessionManagerViewModel: ObservableObject {
    @Published public var sessions: [TmuxSessionInfo] = []
    @Published public var isLoading: Bool = false
    @Published public var errorMessage: String? = nil

    public init() {}

    public func fetchSessions(server: ServerProfile, privateKey: String) {
        guard !server.host.isEmpty, !server.username.isEmpty, !privateKey.isEmpty else {
            errorMessage = "Server host or private key is missing."
            return
        }

        isLoading = true
        errorMessage = nil

        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privateKey
        )

        Task {
            do {
                let fetched = try await listRemoteTmuxSessions(config: config)
                self.sessions = fetched
                self.isLoading = false
            } catch {
                self.errorMessage = "Failed to list tmux sessions: \(error.localizedDescription)"
                self.isLoading = false
            }
        }
    }

    public func killSession(name: String, server: ServerProfile, privateKey: String) {
        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privateKey
        )

        isLoading = true
        errorMessage = nil

        Task {
            do {
                try await killRemoteTmuxSession(config: config, sessionName: name)
                let fetched = try await listRemoteTmuxSessions(config: config)
                self.sessions = fetched
                self.isLoading = false
            } catch {
                self.errorMessage = "Failed to stop session '\(name)': \(error.localizedDescription)"
                self.isLoading = false
            }
        }
    }
}
