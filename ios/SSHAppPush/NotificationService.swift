import Foundation
import OSLog
import SshCoreBridge
import UserNotifications

/// Puts the tab's name into a push on the phone, and for a blocked-host push asks the server
/// which host it was. The push itself carries only a fixed text and an opaque `sid`; the name
/// comes from the list the app published (see `PushNames`), the host from one short SSH exec
/// over the server's pinned key, so neither Apple nor the relay learns either.
/// The system calls one instance from one queue at a time; `done` is guarded by `lock`.
final class NotificationService: UNNotificationServiceExtension, @unchecked Sendable {
    private static let log = Logger(subsystem: "com.semenov.sshapp.push", category: "extension")
    /// What is left of the SSH wait before the plain text goes out (the system allows about 30 s).
    private static let sshSeconds: UInt64 = 20

    private var handler: ((UNNotificationContent) -> Void)?
    private var best: UNMutableNotificationContent?
    private let lock = NSLock()
    private var done = false

    override func didReceive(_ request: UNNotificationRequest,
                             withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        let group = Bundle.main.object(forInfoDictionaryKey: "SSHAppKeychainGroup") as? String
        let info = content.userInfo
        let sid = info["sid"] as? String
        let names = PushNames.load(group: group)
        content.body = PushNames.body(sid: sid, text: content.body, names: names)
        handler = contentHandler
        best = content
        Self.logMemory("start")

        guard info["category"] as? String == "blocked", let sid, let entry = names[sid],
              let serverId = entry.server, let sandbox = entry.sandbox,
              let server = SharedStore.load(group: group).first(where: { $0.id == serverId }),
              !server.guarded, server.bannerLookup else {
            finish(content)
            return
        }
        let engine: SandboxEngine = entry.engine == "podman" ? .podman : .docker
        let text = content.body
        Task {
            keyStoreConfigure(accessGroup: group)
            let config = RemoteServerConfig(
                host: server.host, port: server.port, username: server.username,
                key: KeyRef(id: server.keyId, guarded: false), pinnedHostKey: server.hostKey)
            let hosts = await Self.withTimeout(seconds: Self.sshSeconds) {
                try? await sandboxBlockedHostsFor(config: config, engine: engine, sandboxId: sandbox)
            } ?? []
            Self.logMemory("after ssh (\(hosts.count) hosts)")
            self.finishBlocked(name: entry.name, hosts: hosts, text: text)
        }
    }

    /// The system is about to stop the extension: the best text so far goes out.
    override func serviceExtensionTimeWillExpire() {
        if let best { finish(best) }
    }

    private func finishBlocked(name: String, hosts: [String], text: String) {
        guard let base = best else { return }
        let out = (base.mutableCopy() as? UNMutableNotificationContent) ?? base
        out.body = PushNames.blockedBody(name: name, hosts: hosts, text: text)
        finish(out)
    }

    private func finish(_ content: UNNotificationContent) {
        lock.lock()
        let first = !done
        done = true
        let h = handler
        lock.unlock()
        if first { h?(content) }
    }

    private static func withTimeout<T: Sendable>(seconds: UInt64, _ work: @escaping @Sendable () async -> T?) async -> T? {
        await withTaskGroup(of: T?.self) { group in
            group.addTask { await work() }
            group.addTask {
                try? await Task.sleep(nanoseconds: seconds * 1_000_000_000)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
    }

    /// The memory the extension may still use before the system stops it (about 24 MB in all).
    private static func logMemory(_ when: String) {
        let left = Double(os_proc_available_memory()) / 1_048_576
        log.info("memory left \(when, privacy: .public): \(left, format: .fixed(precision: 1), privacy: .public) MB")
    }
}
