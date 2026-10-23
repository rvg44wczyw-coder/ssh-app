import SwiftUI
import SshCoreBridge

/// Reads the model usage of the current tab from the host (one read-only probe, own SSH
/// connection) so the main screen can show it. Only runs while the app is in the foreground.
@MainActor
final class SessionUsageMonitor: ObservableObject {
    /// A monitor with nothing in it, for lines that have no usage numbers to show.
    static let none = SessionUsageMonitor()
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
                    // With room to spare the line also says how many tokens that is.
                    ViewThatFits(in: .horizontal) {
                        Text("\(Int(frac * 100))% ctx \(tokens(used))/\(tokens(limit))")
                        Text("\(Int(frac * 100))% ctx")
                    }
                    .lineLimit(1)
                    .foregroundColor(frac < 0.9 ? .secondary : .red)
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

/// The one status line, the same above the terminal and above the feed: git (branch, worktree,
/// changes), the agent's model, context and limits, the subagents, refresh, and the icon that
/// switches the tab between terminal and feed. Tap refreshes; long press opens the git sheets.
struct SessionStatusLine: View {
    struct SwitchButton { let icon: String; let title: String; let hint: String }

    @ObservedObject var viewModel: TerminalSessionViewModel
    @ObservedObject var usage: SessionUsageMonitor
    var onRefresh: (() -> Void)?
    var switchButton: SwitchButton?
    @State private var gitNamesFull = false
    @AppStorage("debug_feed_timing") private var showTiming = false
    private enum GitSheet: String, Identifiable { case changes, branches, worktrees; var id: String { rawValue } }
    @State private var gitSheet: GitSheet?
    @State private var showSubagents = false

    private var subagentCounts: (running: Int, total: Int) {
        let calls = viewModel.transcriptTurns.flatMap(\.toolCalls).filter { $0.toolName == "Task" || $0.toolName == "Agent" || $0.toolName == "invoke_subagent" }
        return (calls.filter { $0.status == .running }.count, calls.count)
    }

    var body: some View {
        let git = viewModel.gitContext
        let showsUsage = SessionUsageItems.hasContent(usage)
        return HStack(spacing: 8) {
            if !showsUsage {
                Text(viewModel.feedAgentName).foregroundColor(.secondary)
            }
            if git.isGit, let branch = gitNamesFull ? git.branch : git.shortBranch {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.triangle.branch")
                    Text(branch).lineLimit(gitNamesFull ? 2 : 1).truncationMode(.middle)
                        .fixedSize(horizontal: !gitNamesFull, vertical: false)
                    if git.isWorktree, let wt = gitNamesFull ? git.worktreeName : git.shortWorktree {
                        Image(systemName: "square.stack.3d.down.right")
                        Text(wt).lineLimit(gitNamesFull ? 2 : 1).truncationMode(.middle)
                            .fixedSize(horizontal: !gitNamesFull, vertical: false)
                    }
                    if viewModel.gitDirtyCount > 0 {
                        Text("+\(viewModel.gitDirtyCount)").foregroundColor(.orange)
                    }
                }
                .foregroundColor(.secondary)
                // The branch keeps its width; the model name gives way first.
                .layoutPriority(3)
                .onTapGesture { withAnimation { gitNamesFull.toggle() } }
            }
            SessionUsageItems(monitor: usage)
            if let mode = AgentModeBadge.current(in: viewModel.transcriptTurns) {
                Label(mode.title, systemImage: mode.icon)
                    .labelStyle(.titleAndIcon)
                    .foregroundColor(mode.color)
                    .fixedSize()
                    .layoutPriority(2)
                    .accessibilityLabel("Mode: \(mode.title)")
            }
            if showTiming, let t = viewModel.feedTiming {
                Text("\(t.captureMs)/\(t.fileMs.map(String.init) ?? "-")/\(t.cycleMs)ms")
                    .foregroundColor(.orange)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .layoutPriority(0)
            }
            let agents = subagentCounts
            if agents.total > 0 {
                Button { showSubagents = true } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "person.2.fill")
                        Text(agents.running > 0 ? "\(agents.running) agent\(agents.running == 1 ? "" : "s")" : "\(agents.total) ran")
                    }
                    .frame(minHeight: 22)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundColor(agents.running > 0 ? .accentColor : .secondary)
                .fixedSize()
                .layoutPriority(2)
                .accessibilityHint("Shows the subagents")
            }
            Spacer(minLength: 0)
            Button {
                onRefresh?()
                Task { await viewModel.loadGitContext() }
            } label: {
                if usage.loading || viewModel.isTranscriptLoading {
                    ProgressView().controlSize(.mini)
                } else {
                    Image(systemName: "arrow.clockwise")
                }
            }
            .buttonStyle(.plain)
            .foregroundColor(.secondary)
            .frame(minWidth: 24, minHeight: 22)
            .accessibilityLabel("Refresh")
            if let sw = switchButton {
                Button { viewModel.toggleChatMode() } label: {
                    Image(systemName: sw.icon)
                        .font(.system(size: 12, weight: .semibold))
                        .frame(width: 30, height: 22)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundColor(.accentColor)
                .accessibilityLabel(sw.title)
                .accessibilityHint(sw.hint)
            }
        }
        .font(.system(size: 10, design: .monospaced))
        .padding(.horizontal, 10)
        .frame(minHeight: 22)
        .frame(maxWidth: .infinity)
        .containerRelativeFrame(.horizontal)
        .clipped()
        .background(.bar)
        .contentShape(Rectangle())
        .onTapGesture { onRefresh?(); Task { await viewModel.loadGitContext() } }
        .contextMenu {
            if git.isGit {
                Button { gitSheet = .branches } label: { Label("Branches", systemImage: "arrow.triangle.branch") }
                Button { gitSheet = .worktrees } label: { Label("Worktrees", systemImage: "square.stack.3d.down.right") }
                Button { gitSheet = .changes } label: { Label("Changes", systemImage: "square.and.arrow.up") }
            }
        }
        .sheet(item: $gitSheet) { sheet in
            switch sheet {
            case .changes: ProjectSheetView(viewModel: viewModel, tab: .changes)
            case .branches: ProjectSheetView(viewModel: viewModel, tab: .branches)
            case .worktrees: GitWorktreesSheetView(viewModel: viewModel)
            }
        }
        .sheet(isPresented: $showSubagents) { SubagentsSheetView(viewModel: viewModel) }
        .task { await viewModel.loadGitContext() }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Tap to refresh")
    }
}

extension TranscriptTurn {
    /// Label of the hidden turn that carries the agent's permission mode (see the Rust parser).
    static let permissionModeLabel = "permission-mode"
}

/// The mode the CLI agent runs in, from the last mode record of its session file.
struct AgentModeBadge {
    let title: String
    let icon: String
    let color: Color

    static func current(in turns: [TranscriptTurn]) -> AgentModeBadge? {
        guard let raw = turns.last(where: { $0.label == TranscriptTurn.permissionModeLabel })?.content else { return nil }
        switch raw {
        case "default": return AgentModeBadge(title: "Ask", icon: "hand.raised", color: .secondary)
        case "plan": return AgentModeBadge(title: "Plan", icon: "list.bullet.clipboard", color: .blue)
        case "acceptEdits": return AgentModeBadge(title: "Edits", icon: "pencil", color: .green)
        case "auto": return AgentModeBadge(title: "Auto", icon: "bolt", color: .purple)
        case "bypassPermissions": return AgentModeBadge(title: "Bypass", icon: "exclamationmark.shield", color: .red)
        case "dontAsk": return AgentModeBadge(title: "Don't ask", icon: "bolt.slash", color: .orange)
        default: return AgentModeBadge(title: raw, icon: "gearshape", color: .secondary)
        }
    }
}
