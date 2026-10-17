import Foundation
import Combine
import SshCoreBridge

@MainActor
public final class TmuxSessionManagerViewModel: ObservableObject {
    @Published public var sessions: [TmuxSessionInfo] = []
    @Published public var isLoading: Bool = false
    @Published public var errorMessage: String? = nil
    /// Models running in the sessions (by tmux session name) and rolling token totals.
    @Published public var usage: [String: SessionUsage] = [:]
    @Published public var usageReport: HostUsageReport? = nil
    @Published public var usageError: String? = nil

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
            privateKeyPem: privateKey,
            pinnedHostKey: server.hostKeyFingerprint
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

    /// Best effort: the session list must stay usable if the usage probe fails (no python3 on the host, ...).
    public func fetchUsage(server: ServerProfile, privateKey: String, configDirs: [String]) {
        guard !server.host.isEmpty, !server.username.isEmpty, !privateKey.isEmpty else { return }
        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privateKey,
            pinnedHostKey: server.hostKeyFingerprint
        )
        Task {
            do {
                let report = try await fetchHostUsage(config: config, configDirs: configDirs)
                self.usageReport = report
                self.usage = Dictionary(report.sessions.map { ($0.session, $0) }, uniquingKeysWith: { a, _ in a })
                self.usageError = nil
            } catch {
                self.usage = [:]
                self.usageReport = nil
                self.usageError = error.localizedDescription
            }
        }
    }

    public func killSession(name: String, server: ServerProfile, privateKey: String) {
        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privateKey,
            pinnedHostKey: server.hostKeyFingerprint
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
