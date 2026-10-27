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
              KeychainManager.shared.hasPrivateKey(id: server.keyId) else { return }
        if !force, let updated, Date().timeIntervalSince(updated) < minAge, report != nil { return }
        loading = true
        let config = RemoteServerConfig(host: server.host, port: server.port, username: server.username,
                                        key: KeychainManager.shared.keyRef(server.keyId), pinnedHostKey: server.hostKeyFingerprint)
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

/// The numbers of the strip: model, context percent, plan limits, 5-hour tokens. Shared by the strip
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
                Text("\(Int(frac * 100))% ctx")
                    .foregroundColor(frac < 0.7 ? .secondary : (frac < 0.9 ? .orange : .red))
                    // The numbers never give way to the names around them (the model name and the
                    // git branch are the ones that shrink): "NN% ctx" stays whole.
                    .fixedSize()
                    .layoutPriority(2)
            }
            ForEach(u.limits, id: \.label) { l in
                let color: Color = l.percent < 70 ? .secondary : (l.percent < 90 ? .orange : .red)
                HStack(spacing: 4) {
                    // The 5-hour window gets the fill bar; the others stay a number.
                    if l.label == "5h" {
                        ProgressView(value: min(1, Double(l.percent) / 100))
                            .tint(l.percent < 70 ? .green : (l.percent < 90 ? .orange : .red))
                            .frame(width: 40)
                    }
                    Text("\(l.label) \(l.percent)%").foregroundColor(color)
                }
                .fixedSize()
                .layoutPriority(2)
            }
            if u.limits.isEmpty, AgentResolver.entry(hookName: u.agent).fiveHourEstimate, let w = monitor.report?.fiveHour, w.messages > 0 {
                Text("5h \(tokens(w.input + w.output + w.cacheWrite))").foregroundColor(.secondary)
                    .fixedSize()
                    .layoutPriority(2)
            }
        }
    }
}

/// Whether the status line shows two rows. Kept in memory for the life of the app, shared by every tab.
@MainActor
final class StatusLineLayout: ObservableObject {
    static let shared = StatusLineLayout()
    @Published var expanded = false
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
    @ObservedObject private var layout = StatusLineLayout.shared
    @AppStorage("debug_feed_timing") private var showTiming = false
    private enum GitSheet: String, Identifiable { case changes, branches, worktrees; var id: String { rawValue } }
    @State private var gitSheet: GitSheet?
    @State private var showSubagents = false

    private var subagentCounts: (running: Int, total: Int) {
        let calls = viewModel.transcriptTurns.flatMap(\.toolCalls).filter { $0.toolName == "Task" || $0.toolName == "Agent" || $0.toolName == "invoke_subagent" }
        return (calls.filter { $0.status == .running }.count, calls.count)
    }

    @ViewBuilder private func gitBlock(_ git: GitSessionContext) -> some View {
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
    }

    @ViewBuilder private var modeBadge: some View {
        if let mode = AgentModeBadge.current(in: viewModel.transcriptTurns) {
            Label(mode.title, systemImage: mode.icon)
                .labelStyle(.titleAndIcon)
                .foregroundColor(mode.color)
                .fixedSize()
                .layoutPriority(2)
                .accessibilityLabel("Mode: \(mode.title)")
        }
    }

    @ViewBuilder private var timingBadge: some View {
        if showTiming, let t = viewModel.feedTiming {
            Text("\(t.captureMs)/\(t.fileMs.map(String.init) ?? "-")/\(t.cycleMs)ms")
                .foregroundColor(.orange)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .layoutPriority(0)
        }
    }

    @ViewBuilder private var subagentsButton: some View {
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
    }

    @ViewBuilder private var tasksButton: some View {
        if let tasks = viewModel.backgroundTasks, tasks.count > 0 {
            Button { viewModel.openBackgroundTasks() } label: {
                HStack(spacing: 3) {
                    Image(systemName: "gearshape.2.fill")
                    Text(tasks.count == 1 ? "1 task" : "\(tasks.count) tasks")
                }
                .frame(minHeight: 22)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundColor(viewModel.canOpenBackgroundTasks ? .accentColor : .secondary)
            .disabled(!viewModel.canOpenBackgroundTasks)
            .fixedSize()
            .layoutPriority(2)
            .accessibilityHint("Opens the agent's background tasks")
        }
    }

    @ViewBuilder private var switchControl: some View {
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

    private var expandButton: some View {
        Button { withAnimation(.snappy) { layout.expanded.toggle() } } label: {
            Image(systemName: layout.expanded ? "chevron.up" : "chevron.down")
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 26, height: 22)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundColor(.secondary)
        .accessibilityLabel(layout.expanded ? "Collapse status" : "Expand status")
    }

    var body: some View {
        let git = viewModel.gitContext
        let showsUsage = SessionUsageItems.hasContent(usage)
        // The line is laid out inside an overlay of a clear bar: the bar takes exactly the width it is
        // offered, so content that wants more (long branch, model, chips) is clipped instead of
        // making the strip wider than the screen. Collapsed it is one row; expanded, two rows
        // (git, mode and agents above; model, context and limits below).
        return Color.clear.frame(height: layout.expanded ? 46 : 22).containerRelativeFrame(.horizontal).overlay(alignment: .leading) {
            Group {
                if layout.expanded {
                    VStack(spacing: 2) {
                        HStack(spacing: 8) {
                            gitBlock(git)
                            modeBadge
                            subagentsButton
                        tasksButton
                            tasksButton
                            timingBadge
                            Spacer(minLength: 0)
                            switchControl
                            expandButton
                        }
                        HStack(spacing: 8) {
                            if !showsUsage { Text(viewModel.feedAgentName).foregroundColor(.secondary) }
                            SessionUsageItems(monitor: usage)
                            Spacer(minLength: 0)
                        }
                        .frame(height: 22)
                    }
                } else {
                    HStack(spacing: 8) {
                        // Git first for every agent: the same line whatever runs in the tab.
                        gitBlock(git).layoutPriority(2)
                        if !showsUsage { Text(viewModel.feedAgentName).foregroundColor(.secondary) }
                        SessionUsageItems(monitor: usage)
                        modeBadge
                        timingBadge
                        subagentsButton
                        Spacer(minLength: 0)
                        switchControl
                        expandButton
                    }
                }
            }
            .font(.system(size: 10, design: .monospaced))
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .clipped()
        .background(.bar)
        .contentShape(Rectangle())
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
        // The first look can come before the connection is up (a restored tab): ask again once it is.
        .onChange(of: viewModel.state) { _, new in
            if new == .connected { Task { await viewModel.loadGitContext() } }
        }
        // The agent finished a turn: limits, context and the branch may have changed.
        .onChange(of: viewModel.screenStatus == nil) { _, idle in
            if idle { onRefresh?(); Task { await viewModel.loadGitContext() } }
        }
        .accessibilityElement(children: .combine)
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
