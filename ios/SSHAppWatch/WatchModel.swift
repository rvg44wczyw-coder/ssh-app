import Foundation
import WatchConnectivity
import WatchKit

/// Pending approvals as the iPhone published them, and the verdicts sent back. The watch holds no
/// key and talks to no host: it only relays `{id, verdict}` to the phone (Feature 22).
@MainActor
final class WatchModel: NSObject, ObservableObject {
    enum SendState: Equatable {
        case idle
        case sending(String)
        case done(String, WatchReply)
        case failed(String)
    }

    @Published private(set) var items: [WatchApproval] = []
    @Published private(set) var receivedAt = Date()
    @Published private(set) var phoneReachable = false
    @Published private(set) var sendState: SendState = .idle
    private var seq = 0

    override init() {
        super.init()
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    /// Seconds left for `item`, counted down from the moment the list arrived.
    func remaining(_ item: WatchApproval, now: Date = Date()) -> Int {
        max(0, item.expiresIn - Int(now.timeIntervalSince(receivedAt)))
    }

    private func apply(_ dict: [String: Any]) {
        guard let ctx = WatchProtocol.decode(dict) else { return }
        let known = Set(items.map(\.id))
        seq = ctx.seq
        items = ctx.items
        receivedAt = Date()
        if ctx.items.contains(where: { !known.contains($0.id) }) {
            WKInterfaceDevice.current().play(.notification)
        }
    }

    func send(_ item: WatchApproval, approved: Bool) {
        guard WCSession.default.isReachable else {
            sendState = .failed(item.id)
            WKInterfaceDevice.current().play(.failure)
            return
        }
        sendState = .sending(item.id)
        WCSession.default.sendMessage(
            WatchProtocol.verdictMessage(id: item.id, approved: approved, seq: seq),
            replyHandler: { reply in
                let state = (reply["state"] as? String).flatMap(WatchReply.init(rawValue:)) ?? .rejected
                Task { @MainActor in
                    self.sendState = .done(item.id, state)
                    WKInterfaceDevice.current().play(state == .signed ? .success : .failure)
                    if state != .hostUnreachable { self.items.removeAll { $0.id == item.id } }
                }
            },
            errorHandler: { _ in
                Task { @MainActor in
                    self.sendState = .failed(item.id)
                    WKInterfaceDevice.current().play(.failure)
                }
            }
        )
    }
}

extension WatchModel: WCSessionDelegate {
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) {
        let ctx = session.receivedApplicationContext
        let reachable = session.isReachable
        nonisolated(unsafe) let dict = ctx
        Task { @MainActor in
            self.phoneReachable = reachable
            if !dict.isEmpty { self.apply(dict) }
        }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        let reachable = session.isReachable
        Task { @MainActor in self.phoneReachable = reachable }
    }

    nonisolated func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
        nonisolated(unsafe) let dict = applicationContext
        Task { @MainActor in self.apply(dict) }
    }
}
