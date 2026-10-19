import Foundation
import SwiftUI
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#endif

/// One host to poll: tabs on the same server share a single poll.
public struct ApprovalPollTarget {
    public let serverKey: String
    public let serverName: String
    public let viewModel: TerminalSessionViewModel
}

/// Foreground-only poller of the hosts' approval queues (Feature 13). It runs while the scene is
/// active and stops in the background, so the zero-battery rule holds: nothing is polled, and no
/// socket is held, while the app is suspended.
@MainActor
public final class ApprovalsCenter: ObservableObject {
    @Published public private(set) var entries: [ApprovalEntry] = []
    @Published public private(set) var busyIds: Set<String> = []
    @Published public private(set) var errors: [String: String] = [:]
    /// True while the foreground poll runs; the Apple Watch only shows requests while it does.
    @Published public private(set) var isPolling = false

    private var task: Task<Void, Never>?
    private var targets: () -> [ApprovalPollTarget] = { [] }
    private var handles: [String: SshSessionHandle] = [:]
    private var knownApprovalIds: Set<String> = []
    private let activeInterval: Duration = .seconds(2)
    /// Hosts without an approval queue (hooks not installed) are checked far less often.
    private let idleInterval: Duration = .seconds(10)
    private var queueSeen = false

    public init() {}

    /// The request the user should answer first (oldest approval), if any.
    public var current: ApprovalEntry? { entries.first(where: { $0.isApproval }) }
    public var notices: [ApprovalEntry] { entries.filter { !$0.isApproval } }
    public var queuedBehindCurrent: Int { max(0, entries.filter { $0.isApproval }.count - 1) }

    public func start(targets: @escaping () -> [ApprovalPollTarget]) {
        self.targets = targets
        isPolling = true
        task?.cancel()
        task = Task { [weak self] in
            while !Task.isCancelled {
                let started = ContinuousClock.now
                let outcome = await self?.pollOnce()
                // A long poll that came back at once without news (an older host script answers
                // `wait` immediately) must not turn into a busy loop.
                let heldOrChanged = outcome.map { $0.usedWait && ($0.changed || ContinuousClock.now - started >= .seconds(1)) } ?? false
                if !heldOrChanged {
                    let wait = (self?.queueSeen ?? true) ? self?.activeInterval : self?.idleInterval
                    try? await Task.sleep(for: wait ?? .seconds(2))
                }
            }
        }
    }

    public func stop() {
        task?.cancel()
        task = nil
        isPolling = false
    }

    public func pollNow() async { _ = await pollOnce(allowWait: false) }

    /// `usedWait`: the host held the call (long poll); `changed`: the shown entries changed.
    private func pollOnce(allowWait: Bool = true) async -> (usedWait: Bool, changed: Bool) {
        var seenKeys = Set<String>()
        var merged: [ApprovalEntry] = []
        var anyQueue = false
        var usedWait = false
        let all = targets()
        // Long poll only with a single host: one held call would delay the others.
        let singleHost = allowWait && Set(all.map(\.serverKey)).count == 1
        for target in all where !seenKeys.contains(target.serverKey) {
            guard target.viewModel.state == .connected, let handle = target.viewModel.sessionHandle else { continue }
            seenKeys.insert(target.serverKey)
            do {
                let list: ApprovalList
                if singleHost {
                    usedWait = true
                    let known = entries.filter { $0.serverKey == target.serverKey }.map { $0.item.id }
                    list = try await handle.waitPendingApprovals(knownIds: known)
                } else {
                    list = try await handle.listPendingApprovals()
                }
                handles[target.serverKey] = handle
                // host_time is 0 when the host has no approval queue (hooks not installed).
                if list.hostTimeSec > 0 { anyQueue = true }
                let fetched = Date()
                for item in list.items {
                    merged.append(ApprovalEntry(
                        id: "\(target.serverKey)/\(item.id)",
                        serverKey: target.serverKey,
                        serverName: target.serverName,
                        item: item,
                        hostTimeAtFetch: list.hostTimeSec,
                        fetchedAt: fetched
                    ))
                }
            } catch {
                // A transient failure keeps what we already show for that host.
                merged.append(contentsOf: entries.filter { $0.serverKey == target.serverKey })
                anyQueue = true
            }
        }
        if Task.isCancelled { return (usedWait, false) }
        queueSeen = anyQueue || seenKeys.isEmpty
        merged.sort { ($0.item.timestampSec, $0.id) < ($1.item.timestampSec, $1.id) }
        let newApprovals = merged.filter { $0.isApproval && !knownApprovalIds.contains($0.id) }
        knownApprovalIds = Set(merged.filter { $0.isApproval }.map { $0.id })
        let live = Set(merged.map { $0.id })
        errors = errors.filter { live.contains($0.key) }
        let changed = merged != entries
        if changed { entries = merged }
        if !newApprovals.isEmpty { notifyNewApproval() }
        return (usedWait, changed)
    }

    /// Signs and sends the decision. The entry stays until the host stops listing it.
    public func decide(_ entry: ApprovalEntry, approved: Bool) async {
        guard let handle = handles[entry.serverKey], !busyIds.contains(entry.id) else { return }
        busyIds.insert(entry.id)
        errors[entry.id] = nil
        defer { busyIds.remove(entry.id) }
        do {
            try await handle.submitApprovalDecision(id: entry.item.id, approved: approved)
            entries.removeAll { $0.id == entry.id }
            _ = await pollOnce()
        } catch {
            let message = error.localizedDescription
            errors[entry.id] = message
            if message.contains("gone or has expired") {
                entries.removeAll { $0.id == entry.id }
            }
        }
    }

    /// Dismisses an informational notice on the host.
    public func dismiss(_ entry: ApprovalEntry) async {
        guard let handle = handles[entry.serverKey] else { return }
        entries.removeAll { $0.id == entry.id }
        try? await handle.acknowledgeNotification(id: entry.item.id)
    }

    private func notifyNewApproval() {
        #if canImport(UIKit)
        let generator = UINotificationFeedbackGenerator()
        generator.prepare()
        generator.notificationOccurred(.warning)
        #endif
    }
}
