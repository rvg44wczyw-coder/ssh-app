import Foundation
import SshCoreBridge

/// Fetches a host snapshot with one short-lived connection and no terminal session behind it.
///
/// The Host Monitor screen uses it for its periodic refresh, and a home-screen widget can call the
/// same function from its timeline provider: a widget cannot keep a session open, it only needs
/// the server profile, its private key and the pinned host key. (A widget extension additionally
/// needs a shared Keychain access group and a shared store for the server profiles.)
public enum HostMonitorClient {
    public static func fetch(server: ServerProfile, privateKey: String) async throws -> HostTelemetrySnapshot {
        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privateKey,
            pinnedHostKey: server.hostKeyFingerprint
        )
        return try await fetchHostTelemetryOnce(config: config, helperSha256: HostTargetCache.pin(host: server.host, port: server.port, username: server.username))
    }
}

/// Which system each host runs, as seen by the last terminal session to it, so views without a live
/// session (usage strip, host monitor) pin the right helper build. Unknown host: the Mac build.
public final class HostTargetCache: @unchecked Sendable {
    private static let shared = HostTargetCache()
    private let lock = NSLock()
    private var targets: [String: HostTarget] = [:]

    public static func key(host: String, port: UInt16, username: String) -> String {
        "\(username)@\(host):\(port)"
    }

    public static func set(_ target: HostTarget, forKey key: String) {
        shared.lock.lock(); defer { shared.lock.unlock() }
        shared.targets[key] = target
    }

    public static func target(forKey key: String) -> HostTarget? {
        shared.lock.lock(); defer { shared.lock.unlock() }
        return shared.targets[key]
    }

    /// What keeps sessions alive on this host (tmux unless it is known to be native Windows).
    public static func multiplexer(host: String, port: UInt16, username: String) -> MultiplexerKind {
        guard let t = target(forKey: key(host: host, port: port, username: username)) else { return .tmux }
        return hostTargetMultiplexer(target: t)
    }

    public static func pin(host: String, port: UInt16, username: String) -> String? {
        shared.lock.lock(); defer { shared.lock.unlock() }
        guard let target = shared.targets[key(host: host, port: port, username: username)] else {
            return bundledHostHelperSha256()
        }
        return bundledHostHelperSha256For(target: target)
    }
}
