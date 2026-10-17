import Foundation
import Combine
import SshCoreBridge
#if canImport(WatchConnectivity)
import WatchConnectivity
#endif

/// Relays pending approvals to the Apple Watch and the watch's verdicts back (Feature 22).
/// The key never leaves the phone: a verdict from the watch goes through the normal path
/// (`ApprovalsCenter.decide`), which signs the host's own copy of the request.
@MainActor
public final class WatchBridge: NSObject, ObservableObject {
    public static let shared = WatchBridge()

    private weak var center: ApprovalsCenter?
    private var cancellable: AnyCancellable?
    private var seq = 0
    private var lastItems: [WatchApproval] = []

    /// The watch-facing list for the current approvals (pure, for tests).
    static func items(from entries: [ApprovalEntry], now: Date = Date()) -> [WatchApproval] {
        entries.filter { $0.isApproval && WatchProtocol.isValidId($0.item.id) }
            .prefix(WatchProtocol.maxItems)
            .map { e in
                let analysis = analyzeCommand(command: e.item.command)
                let dir = e.item.cwd.split(separator: "/").last.map(String.init) ?? ""
                return WatchApproval(
                    id: e.item.id,
                    agent: WatchProtocol.sanitize(e.item.agent, max: 20),
                    tool: WatchProtocol.sanitize(e.item.tool, max: 30),
                    tail: WatchProtocol.preview(e.item.command).text,
                    truncated: WatchProtocol.preview(e.item.command).truncated,
                    length: e.item.command.count,
                    dir: WatchProtocol.sanitize(dir, max: 40),
                    flags: analysis.flags.prefix(6).map { WatchProtocol.sanitize($0, max: 40) },
                    expiresIn: e.remainingSeconds(at: now),
                    server: WatchProtocol.sanitize(e.serverName, max: 30)
                )
            }
    }

    public func attach(to center: ApprovalsCenter) {
        guard self.center !== center else { return }
        self.center = center
        #if canImport(WatchConnectivity)
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
        cancellable = Publishers.CombineLatest(center.$entries, center.$isPolling)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] entries, polling in
                // Nothing is polled in the background, so the watch must not keep showing stale requests.
                self?.publish(polling ? Self.items(from: entries) : [])
            }
        #endif
    }

    private func publish(_ items: [WatchApproval]) {
        #if canImport(WatchConnectivity)
        guard WCSession.default.activationState == .activated, WCSession.default.isWatchAppInstalled else { return }
        // Countdown changes are not worth a message; only the set of requests is.
        guard items.map(\.id) != lastItems.map(\.id) || items.isEmpty != lastItems.isEmpty else { return }
        lastItems = items
        seq += 1
        try? WCSession.default.updateApplicationContext(WatchProtocol.encode(WatchContext(seq: seq, items: items)))
        #endif
    }

    func handle(verdict message: [String: Any]) async -> WatchReply {
        guard let (id, approved, seq) = WatchProtocol.parseVerdict(message), seq <= self.seq,
              let center else { return .rejected }
        let matches = center.entries.filter { $0.isApproval && $0.item.id == id }
        guard matches.count == 1, let entry = matches.first else { return .notPending }
        guard entry.remainingSeconds() > 0 else { return .expired }
        // A command that did not fit on the watch is not approved from there, whatever the watch says.
        if approved, WatchProtocol.preview(entry.item.command).truncated { return .rejected }
        await center.decide(entry, approved: approved)
        if let err = center.errors[entry.id] {
            return err.contains("gone or has expired") ? .expired : .hostUnreachable
        }
        return .signed
    }
}

#if canImport(WatchConnectivity)
extension WatchBridge: WCSessionDelegate {
    nonisolated public func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) {
        Task { @MainActor in self.lastItems = []; self.republish() }
    }
    nonisolated public func sessionDidBecomeInactive(_ session: WCSession) {}
    nonisolated public func sessionDidDeactivate(_ session: WCSession) { session.activate() }
    nonisolated public func sessionWatchStateDidChange(_ session: WCSession) {
        Task { @MainActor in self.lastItems = []; self.republish() }
    }

    nonisolated public func session(_ session: WCSession, didReceiveMessage message: [String: Any],
                                    replyHandler: @escaping ([String: Any]) -> Void) {
        nonisolated(unsafe) let message = message
        Task { @MainActor in
            replyHandler(WatchProtocol.replyMessage(await self.handle(verdict: message)))
        }
    }
}

extension WatchBridge {
    fileprivate func republish() {
        guard let center else { return }
        publish(center.isPolling ? Self.items(from: center.entries) : [])
    }
}
#endif
