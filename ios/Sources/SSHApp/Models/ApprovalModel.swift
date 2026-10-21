import Foundation
import SshCoreBridge

/// One request or notice from a host's approval queue, with what is needed to show a countdown
/// on the host's clock (the phone clock is never trusted for expiry).
public struct ApprovalEntry: Identifiable, Sendable {
    /// `<server key>/<request id>`: the same id can in theory exist on two hosts.
    public let id: String
    public let serverKey: String
    public let serverName: String
    public let item: PendingApproval
    public let hostTimeAtFetch: UInt64
    public let fetchedAt: Date

    public var isApproval: Bool { item.kind == .approval }

    /// Seconds left before the host-side hook gives up and the agent falls back to its own prompt.
    public func remainingSeconds(at now: Date = Date()) -> Int {
        let hostNow = Double(hostTimeAtFetch) + now.timeIntervalSince(fetchedAt)
        return max(0, Int(Double(item.expiresSec) - hostNow))
    }

    /// Last two path components, so a long working directory stays readable.
    public var shortCwd: String {
        let parts = item.cwd.split(separator: "/")
        guard parts.count > 2 else { return item.cwd }
        return "…/" + parts.suffix(2).joined(separator: "/")
    }
}

extension ApprovalEntry: Equatable {
    /// The countdown base (`hostTimeAtFetch`/`fetchedAt`) is deliberately ignored: an unchanged
    /// request keeps its first consistent base instead of republishing on every poll.
    public static func == (lhs: ApprovalEntry, rhs: ApprovalEntry) -> Bool {
        lhs.id == rhs.id && lhs.item == rhs.item && lhs.serverName == rhs.serverName
    }
}
