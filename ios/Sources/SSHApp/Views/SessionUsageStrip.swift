import SwiftUI
import SshCoreBridge

/// Reads the model usage of the current tab from the host (one read-only probe, own SSH
/// connection) so the main screen can show it. Only runs while the app is in the foreground.
@MainActor
final class SessionUsageMonitor: ObservableObject {
    @Published private(set) var usage: SessionUsage?
    @Published private(set) var report: HostUsageReport?
    @Published private(set) var loading = false
    @Published private(set) var updated: Date?
    private var shownSession: String?
    /// Last known usage of every session seen in a probe, so switching tabs shows something at
    /// once and the probe only refreshes it.
    private var lastKnown: [String: SessionUsage] = [:]

    /// Skips the probe when the data is younger than `minAge` seconds (unless `force`).
    func refresh(sessionName: String, server: ServerProfile?, configDirs: [String], force: Bool = false, minAge: TimeInterval = 60) {
        if shownSession != sessionName { shownSession = sessionName }
        usage = lastKnown[sessionName] ?? report?.sessions.first { $0.session == sessionName }
        guard !loading, let server, !server.host.isEmpty, !server.username.isEmpty,
              let key = KeychainManager.shared.getPrivateKey(id: server.keyId), !key.isEmpty else { return }
        if !force, let updated, Date().timeIntervalSince(updated) < minAge, report != nil { return }
        loading = true
        let config = RemoteServerConfig(host: server.host, port: server.port, username: server.username,
                                        privateKeyPem: key, pinnedHostKey: server.hostKeyFingerprint)
        Task {
            defer { loading = false }
            guard let r = try? await fetchHostUsage(config: config, configDirs: configDirs, helperSha256: HostTargetCache.pin(host: server.host, port: server.port, username: server.username)) else { return }
            report = r
            updated = Date()
            for s in r.sessions { lastKnown[s.session] = s }
            // A session that no longer runs a model must not keep showing its old numbers.
            let running = Set(r.sessions.map(\.session))
            lastKnown = lastKnown.filter { running.contains($0.key) }
            if let shownSession { usage = lastKnown[shownSession] }
        }
    }
}

/// The numbers of the strip: model, context fill, plan limits, 5-hour tokens. Shared by the strip
/// under the tabs and the one line of the agent feed.
struct SessionUsageItems: View {
    @ObservedObject var monitor: SessionUsageMonitor

    private func tokens(_ n: UInt64) -> String {
        n >= 1_000_000 ? String(format: "%.1fM", Double(n) / 1e6) : n >= 1_000 ? String(format: "%.0fk", Double(n) / 1e3) : "\(n)"
    }

    private func clock(_ epoch: UInt64) -> String {
        Date(timeIntervalSince1970: TimeInterval(epoch)).formatted(date: .omitted, time: .shortened)
    }

    static func hasContent(_ monitor: SessionUsageMonitor) -> Bool {
        monitor.usage?.agent != nil || monitor.usage?.limitNote != nil
    }

    var body: some View {
        if let u = monitor.usage, Self.hasContent(monitor) {
            if let note = u.limitNote {
                Label {
                    Text(u.limitResetEpoch.map { "Limit reached · resets \(clock($0))" } ?? (note.components(separatedBy: "|").first ?? note))
                        .lineLimit(1)
                } icon: { Image(systemName: "exclamationmark.triangle.fill") }
                .foregroundColor(.red)
            } else {
                Text(u.model ?? u.agent ?? "").lineLimit(1).truncationMode(.tail).foregroundColor(.secondary)
                    .layoutPriority(1)
            }
            if let used = u.contextUsed, let limit = u.contextLimit, limit > 0 {
                let frac = min(1, Double(used) / Double(limit))
                HStack(spacing: 4) {
                    ProgressView(value: frac)
                        .tint(frac < 0.7 ? .green : (frac < 0.9 ? .orange : .red))
                        .frame(width: 54)
                    Text("\(Int(frac * 100))% ctx").foregroundColor(frac < 0.9 ? .secondary : .red)
                }
                // The numbers never give way to the names around them (the model name and the
                // git branch are the ones that shrink): "NN% ctx" stays whole.
                .fixedSize()
                .layoutPriority(2)
            }
            ForEach(u.limits, id: \.label) { l in
                Text("\(l.label) \(l.percent)%")
                    .foregroundColor(l.percent < 70 ? .secondary : (l.percent < 90 ? .orange : .red))
                    .fixedSize()
                    .layoutPriority(2)
            }
            if u.limits.isEmpty, u.agent == "claude", let w = monitor.report?.fiveHour, w.messages > 0 {
                Text("5h \(tokens(w.input + w.output + w.cacheWrite))").foregroundColor(.secondary)
                    .fixedSize()
                    .layoutPriority(2)
            }
        }
    }
}

/// One thin line under the tabs: model, context fill, 5-hour tokens and a "limit reached" notice.
/// Tap to refresh.
struct SessionUsageStrip: View {
    @ObservedObject var monitor: SessionUsageMonitor
    var onTap: () -> Void

    var body: some View {
        if SessionUsageItems.hasContent(monitor) {
            Button(action: onTap) {
                HStack(spacing: 8) {
                    SessionUsageItems(monitor: monitor)
                    Spacer(minLength: 0)
                    if monitor.loading { ProgressView().controlSize(.mini) }
                }
                .font(.system(size: 10, design: .monospaced))
                .padding(.horizontal, 10)
                .frame(height: 20)
                .frame(maxWidth: .infinity)
                .background(.bar)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Model usage of this session")
            .accessibilityHint("Double tap to refresh")
            Divider()
        }
    }
}
