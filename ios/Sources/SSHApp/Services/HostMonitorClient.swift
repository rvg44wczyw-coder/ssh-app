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
        return try await fetchHostTelemetryOnce(config: config, helperSha256: bundledHostHelperSha256())
    }
}
