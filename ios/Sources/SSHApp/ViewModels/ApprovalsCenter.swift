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
    /// tmux session of an agent session, for naming a notice's tab: `<server key>/<agent>/<agent session>`.
    @Published public private(set) var tmuxNames: [String: String] = [:]
    @Published public private(set) var errors: [String: String] = [:]
    /// What an approval card may offer as "Approve, and allow … for …": by entry id. Only for a
    /// request whose folder is inside a project of the auto-approval policy and whose command is
    /// covered by categories.
    @Published public private(set) var trustOffers: [String: AutoTrustOffer] = [:]
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
    /// Requests this phone has answered, for a short time. A poll that was already
    /// in flight when the answer was sent still lists the request; without this it would flash
    /// back for a moment. After 30 s a request the host still lists is shown again (it was not taken).
    private var decided: [String: Date] = [:]
    private let decidedGrace: TimeInterval = 30

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
                    // Answered requests count as known, so the host's long poll does not return at once for them.
                    let prefix = target.serverKey + "/"
                    let known = entries.filter { $0.serverKey == target.serverKey }.map { $0.item.id }
                        + decided.keys.filter { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
                    list = try await handle.waitPendingApprovals(knownIds: known)
                } else {
                    list = try await handle.listPendingApprovals()
                }
                handles[target.serverKey] = handle
                // host_time is 0 when the host has no approval queue (hooks not installed).
                if list.hostTimeSec > 0 { anyQueue = true }
                let fetched = Date()
                for item in list.items {
                    let id = "\(target.serverKey)/\(item.id)"
                    if let at = decided[id], Date().timeIntervalSince(at) < decidedGrace { continue }
                    merged.append(ApprovalEntry(
                        id: id,
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
        decided = decided.filter { Date().timeIntervalSince($0.value) < decidedGrace }
        queueSeen = anyQueue || seenKeys.isEmpty
        merged.sort { ($0.item.timestampSec, $0.id) < ($1.item.timestampSec, $1.id) }
        let newApprovals = merged.filter { $0.isApproval && !knownApprovalIds.contains($0.id) }
        knownApprovalIds = Set(merged.filter { $0.isApproval }.map { $0.id })
        let live = Set(merged.map { $0.id })
        errors = errors.filter { live.contains($0.key) }
        let changed = merged != entries
        // Names first, so a notice appears with its tab already known.
        if changed { await resolveNoticeTabs(all, merged) }
        if changed { entries = merged }
        if !newApprovals.isEmpty { notifyNewApproval() }
        trustOffers = trustOffers.filter { live.contains($0.key) }
        await loadTrustOffers(newApprovals)
        return (usedWait, changed)
    }

    /// A notice names its agent session only by the agent's own id; the host agent knows which tmux
    /// session that is, so the notice can show the tab. Asked once per host when a notice lacks it.
    private func resolveNoticeTabs(_ targets: [ApprovalPollTarget], _ merged: [ApprovalEntry]) async {
        for target in targets {
            let missing = merged.contains {
                $0.serverKey == target.serverKey && tmuxNames[tmuxKey($0)] == nil
            }
            guard missing, let handle = handles[target.serverKey],
                  let pin = await target.viewModel.helperPin(),
                  let state = try? await handle.hostAgentState(expectedSha256: pin) else { continue }
            for s in state.sessions {
                if let tmux = s.tmux { tmuxNames["\(target.serverKey)/\(s.agent)/\(s.session)"] = tmux }
            }
        }
    }

    private func tmuxKey(_ e: ApprovalEntry) -> String { "\(e.serverKey)/\(e.item.agent)/\(e.item.session)" }

    /// The tmux session of the agent a notice is about, when the host agent has told us.
    public func tmuxName(for e: ApprovalEntry) -> String? { tmuxNames[tmuxKey(e)] }

    /// Signs and sends the decision. The entry stays until the host stops listing it.
    public func decide(_ entry: ApprovalEntry, approved: Bool) async {
        guard let handle = handles[entry.serverKey], !busyIds.contains(entry.id) else { return }
        busyIds.insert(entry.id)
        errors[entry.id] = nil
        defer { busyIds.remove(entry.id) }
        do {
            try await handle.submitApprovalDecision(id: entry.item.id, approved: approved)
            decided[entry.id] = Date()
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

    /// Reads each host's policy once per batch of new requests and works out the offers.
    private func loadTrustOffers(_ fresh: [ApprovalEntry]) async {
        var policies: [String: AutoPolicy] = [:]
        for e in fresh {
            guard let handle = handles[e.serverKey] else { continue }
            if policies[e.serverKey] == nil {
                policies[e.serverKey] = (try? await handle.readHostGuard())?.auto
            }
            guard let policy = policies[e.serverKey], !e.item.session.isEmpty,
                  let offer = autoTrustOffer(policy: policy, tool: e.item.tool, command: e.item.command, cwd: e.item.cwd)
            else { continue }
            trustOffers[e.id] = offer
        }
    }

    /// Approves and trusts `categories` in the offer's project for `seconds` (same agent session).
    public func decideWithTrust(_ entry: ApprovalEntry, categories: [String], seconds: UInt32) async {
        guard let handle = handles[entry.serverKey], !busyIds.contains(entry.id) else { return }
        busyIds.insert(entry.id)
        errors[entry.id] = nil
        defer { busyIds.remove(entry.id) }
        do {
            try await handle.approveWithTrust(id: entry.item.id, categoryIds: categories, trustSecs: seconds)
            decided[entry.id] = Date()
            entries.removeAll { $0.id == entry.id }
            _ = await pollOnce()
        } catch {
            errors[entry.id] = error.localizedDescription
            if error.localizedDescription.contains("gone or has expired") {
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
