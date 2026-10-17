import Foundation
import FileProvider
import SshCoreBridge

/// One SSH connection per server, opened on demand and closed after a short idle window, so there
/// is never a long-lived socket (Zero Battery Drain) but a folder with many requests is not slowed
/// down by a handshake each time.
actor ConnectionPool {
    private let server: SharedServer
    private let keyGroup: String?
    private var client: RemoteFsClient?
    private var rootCache: String?
    private var idleTask: Task<Void, Never>?
    private var users = 0
    private static let idleSeconds: UInt64 = 15

    init(server: SharedServer, keyGroup: String?) {
        self.server = server
        self.keyGroup = keyGroup
    }

    private func makeClient() throws -> RemoteFsClient {
        guard let pem = SharedStore.privateKey(id: server.keyId, group: keyGroup) else {
            throw NSFileProviderError(.notAuthenticated)
        }
        return try RemoteFsClient(config: RemoteServerConfig(
            host: server.host, port: server.port, username: server.username,
            privateKeyPem: pem, pinnedHostKey: server.hostKey
        ))
    }

    /// Runs `body` with the connection; the idle timer restarts when the last user is done.
    func withClient<T>(_ body: (RemoteFsClient, String) async throws -> T) async throws -> T {
        idleTask?.cancel()
        users += 1
        defer {
            users -= 1
            if users == 0 { scheduleClose() }
        }
        let c: RemoteFsClient
        if let existing = client {
            c = existing
        } else {
            c = try makeClient()
            client = c
        }
        let root = try await rootPath(c)
        return try await body(c, root)
    }

    private func rootPath(_ c: RemoteFsClient) async throws -> String {
        if let rootCache { return rootCache }
        let folder: String
        if let start = server.startFolder, !start.isEmpty {
            folder = start
        } else {
            folder = try await c.homeDirectory()
        }
        // The root has to be a real folder the policy allows; `statRemote` answers with the resolved path.
        let entry = try await c.statRemote(path: folder)
        rootCache = entry.path
        return entry.path
    }

    private func scheduleClose() {
        idleTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.idleSeconds * 1_000_000_000)
            guard !Task.isCancelled else { return }
            await self?.close()
        }
    }

    func close() async {
        idleTask?.cancel()
        let c = client
        client = nil
        await c?.close()
    }
}
