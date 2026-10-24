import Foundation
import SshCoreBridge

/// The command connection of a server that has no connected session (feature 54): the server-level
/// screens (containers, processes, ports, monitor) run on it. It is a `TerminalSessionViewModel`
/// without a terminal, so the screens work on it as they do on a session's.
///
/// Zero Battery Drain: a connection exists only while a screen of the server is open, closes
/// `idleSeconds` after the last one is closed, and at once when the app goes to the background.
/// It is never opened for a host key that is not pinned.
@MainActor
final class ServerConnections {
    static let shared = ServerConnections()
    static let idleSeconds: UInt64 = 15

    private var models: [UUID: TerminalSessionViewModel] = [:]
    private var holders: [UUID: Int] = [:]
    private var closers: [UUID: Task<Void, Never>] = [:]
    private var profiles: [UUID: ServerProfile] = [:]

    /// A server can be opened without a session when it has a key and a pinned host key.
    static func canOpen(_ profile: ServerProfile?, keychain: KeychainManager = .shared) -> Bool {
        guard let profile, let pin = profile.hostKeyFingerprint, !pin.isEmpty else { return false }
        return keychain.getPrivateKey(id: profile.keyId) != nil
    }

    /// Opens (or reuses) the connection for a screen and counts the screen as a holder; pair with
    /// `release`. Nil when the server cannot be opened; a connection that failed is returned with its
    /// failed state so the screen can say why.
    func acquire(_ profile: ServerProfile) async -> TerminalSessionViewModel? {
        guard Self.canOpen(profile), let pem = KeychainManager.shared.getPrivateKey(id: profile.keyId) else { return nil }
        closers[profile.id]?.cancel()
        closers[profile.id] = nil
        holders[profile.id, default: 0] += 1
        profiles[profile.id] = profile
        let vm: TerminalSessionViewModel
        if let existing = models[profile.id] {
            vm = existing
        } else {
            vm = TerminalSessionViewModel(session: TabSession(title: profile.name, serverId: profile.id))
            vm.pinnedHostKey = profile.hostKeyFingerprint
            models[profile.id] = vm
        }
        await vm.connectForServer(host: profile.host, port: profile.port, username: profile.username, privateKeyPEM: pem)
        return vm
    }

    func release(_ server: UUID) {
        guard let n = holders[server] else { return }
        holders[server] = max(0, n - 1)
        guard holders[server] == 0 else { return }
        closers[server]?.cancel()
        closers[server] = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.idleSeconds * 1_000_000_000)
            guard !Task.isCancelled else { return }
            self?.close(server)
        }
    }

    private func close(_ server: UUID) {
        closers[server] = nil
        models[server]?.disconnectServer()
        models[server] = nil
    }

    /// The app is going to the background: no socket stays open.
    func closeAll() {
        for task in closers.values { task.cancel() }
        closers = [:]
        for (_, vm) in models { vm.disconnectServer() }
    }

    /// Back in the foreground: a screen that is still open gets its connection again.
    func reconnectOpen() async {
        for (id, vm) in models where (holders[id] ?? 0) > 0 {
            guard let profile = profiles[id], let pem = KeychainManager.shared.getPrivateKey(id: profile.keyId) else { continue }
            await vm.connectForServer(host: profile.host, port: profile.port, username: profile.username, privateKeyPEM: pem)
        }
        // Models nobody holds were closed in the background; forget them.
        for id in models.keys where (holders[id] ?? 0) == 0 { models[id] = nil }
    }
}
