import SwiftUI
import SshCoreBridge

/// History of the repository the active terminal tab is working in. A tap opens the patch of a
/// commit; the working-tree commands run in the terminal itself (with `!` inside an AI CLI).
public struct GitLogSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel

    @State private var result: GitLogResult? = nil
    @State private var isLoading = true
    @State private var errorMessage: String? = nil
    @State private var limit: UInt32 = 50
    @State private var selected: GitLogEntry? = nil
    @State private var showWorkingTree = false
    @State private var showChanges = false
    @State private var context: PaneContext? = nil
    // Commit operations (feature 28, G3).
    @State private var selecting = false
    @State private var picked: Set<String> = []
    @State private var pending: PendingCommitOp?
    @State private var reword: GitLogEntry?
    @State private var rewordText = ""
    @State private var rewordIsTip = true
    @State private var opMessage: String?
    @State private var squashCount: Int?
    @State private var squashText = ""
    @State private var squashRange: (oldest: String, newest: String)?
    @State private var opFailed = false

    public init(viewModel: TerminalSessionViewModel) {
        self.viewModel = viewModel
    }

    private var inAiCli: Bool { context?.kind == .aiCli }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                commandBar
                Divider()
                content
            }
            .navigationTitle("Git log")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    HStack {
                        Button(selecting ? "Cancel" : "Select") {
                            selecting.toggle()
                            picked = []
                        }
                        Button(action: fetchLog) { Image(systemName: "arrow.clockwise") }.disabled(isLoading)
                    }
                }
            }
            .sheet(item: $selected) { entry in
                DiffViewerSheetView(viewModel: viewModel, commit: entry)
            }
            .sheet(isPresented: $showChanges, onDismiss: fetchLog) {
                GitChangesSheetView(viewModel: viewModel)
            }
            .sheet(isPresented: $showWorkingTree) {
                DiffViewerSheetView(viewModel: viewModel)
            }
            .safeAreaInset(edge: .bottom) { pickBar }
            .modifier(CommitDialogs(
                pending: $pending, squashCount: $squashCount, squashText: $squashText,
                squashRange: $squashRange, reword: $reword, rewordText: $rewordText, rewordIsTip: rewordIsTip, run: runOp))
            .onAppear {
                fetchLog()
                Task { context = await viewModel.currentPaneContext() }
            }
        }
    }

    /// Buttons that type the command into the terminal.
    private var commandBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(["git diff", "git diff --staged", "git status -sb"], id: \.self) { cmd in
                        Button {
                            viewModel.runShellCommand(cmd)
                            dismiss()
                        } label: {
                            Text(verbatim: cmd).font(.system(size: 12, design: .monospaced))
                        }
                        .buttonStyle(.bordered)
                    }
                }
            }
            if inAiCli {
                Text("An AI CLI is running here (\(context?.agent ?? "")): commands are sent with a leading !")
                    .font(.system(size: 11)).foregroundColor(.secondary)
            }
        }
        .padding(10)
    }

    @ViewBuilder
    private var content: some View {
        if isLoading && result == nil {
            VStack(spacing: 12) {
                ProgressView()
                Text("Reading the history from the host...").font(.system(size: 13)).foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let error = errorMessage {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.triangle").font(.system(size: 32)).foregroundColor(.orange)
                Text(error).font(.system(size: 13)).foregroundColor(.secondary)
                    .multilineTextAlignment(.center).padding(.horizontal)
                Button("Retry") { fetchLog() }.buttonStyle(.bordered)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let r = result, r.status != .ok {
            VStack(spacing: 10) {
                Image(systemName: "folder.badge.questionmark").font(.system(size: 34)).foregroundColor(.orange)
                switch r.status {
                case .notARepo:
                    Text("Not a git repository").font(.system(size: 14, weight: .semibold))
                    if let dir = r.directory {
                        Text(verbatim: "This terminal is in \(dir). cd into a repository and refresh.")
                            .font(.system(size: 12)).foregroundColor(.secondary)
                            .multilineTextAlignment(.center).padding(.horizontal)
                    }
                case .noSession:
                    Text("Terminal session not found on the host").font(.system(size: 14, weight: .semibold))
                default:
                    Text("Unexpected answer from the host").font(.system(size: 14, weight: .semibold))
                }
                Button("Refresh") { fetchLog() }.buttonStyle(.bordered)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let r = result {
            List {
                Section {
                    if let opMessage {
                        Text(opMessage).font(.system(size: 12, design: .monospaced))
                            .foregroundColor(opFailed ? .red : .secondary)
                    }
                    Button { showChanges = true } label: {
                        Label("Stage, commit, push…", systemImage: "square.and.arrow.up")
                            .font(.system(size: 14, weight: .medium))
                    }
                    Button { showWorkingTree = true } label: {
                        Label("Uncommitted changes", systemImage: "pencil.and.list.clipboard")
                            .font(.system(size: 14, weight: .medium))
                    }
                    ForEach(Array(r.entries.enumerated()), id: \.element.hash) { idx, e in
                        Button {
                            if selecting {
                                if picked.contains(e.hash) { picked.remove(e.hash) } else { picked.insert(e.hash) }
                            } else {
                                selected = e
                            }
                        } label: {
                            HStack(spacing: 8) {
                                if selecting {
                                    Image(systemName: picked.contains(e.hash) ? "checkmark.circle.fill" : "circle")
                                        .foregroundColor(picked.contains(e.hash) ? .accentColor : .secondary)
                                }
                                row(e)
                            }
                        }
                        .buttonStyle(.plain)
                        .contextMenu { commitMenu(e, isTip: idx == 0) }
                    }
                    if r.hasMore {
                        Button("Show more") {
                            limit += 50
                            fetchLog()
                        }
                    }
                } header: {
                    if let root = r.repoRoot {
                        Text(verbatim: "\(root)\(r.branch.map { "  (\($0))" } ?? "")")
                            .font(.system(size: 11, design: .monospaced))
                    }
                }
            }
            .listStyle(.plain)
        }
    }

    @ViewBuilder
    private func commitMenu(_ e: GitLogEntry, isTip: Bool) -> some View {
        Button {
            pending = PendingCommitOp(
                title: "Revert \(e.shortHash)?", button: "Revert", destructive: false,
                message: "Adds a new commit that undoes \"\(e.subject)\". Needs a clean tree; a conflict aborts it.",
                op: .commit(.revert(hash: e.hash)))
        } label: { Label("Revert", systemImage: "arrow.uturn.backward") }
        Button {
            pending = PendingCommitOp(
                title: "Cherry-pick \(e.shortHash)?", button: "Cherry-pick", destructive: false,
                message: "Applies \"\(e.subject)\" on top of the current branch. A conflict aborts it and changes nothing.",
                op: .commit(.cherryPick(hashes: [e.hash])))
        } label: { Label("Cherry-pick here", systemImage: "arrow.down.to.line") }
        if !isTip {
            Button {
                pending = PendingCommitOp(
                    title: "Move the branch to \(e.shortHash)?", button: "Reset (keep changes staged)", destructive: true,
                    message: "The newer commits leave the branch but their changes stay staged. The old tip is saved under refs/ssh-app/backup/.",
                    op: .commit(.reset(hash: e.hash, mode: .soft)))
            } label: { Label("Reset to here (soft)", systemImage: "arrow.counterclockwise") }
            Button {
                pending = PendingCommitOp(
                    title: "Move the branch to \(e.shortHash)?", button: "Reset (unstage)", destructive: true,
                    message: "The newer commits leave the branch; their changes stay in your files, unstaged. The old tip is saved under refs/ssh-app/backup/.",
                    op: .commit(.reset(hash: e.hash, mode: .mixed)))
            } label: { Label("Reset to here (mixed)", systemImage: "arrow.counterclockwise") }
        }
        Button {
            rewordText = e.subject
            rewordIsTip = isTip
            reword = e
        } label: { Label("Reword message", systemImage: "pencil") }
        Button(role: .destructive) {
            pending = PendingCommitOp(
                title: "Drop \(e.shortHash)?", button: "Drop commit", destructive: true,
                message: "Removes \"\(e.subject)\" from this branch; the later commits are replayed. Refused if it is already on a remote or if a replay conflicts. The old tip is saved under refs/ssh-app/backup/.",
                op: .history(.drop(hash: e.hash)))
        } label: { Label("Drop commit", systemImage: "trash") }
        Button { UIPasteboard.general.string = e.hash } label: { Label("Copy hash", systemImage: "doc.on.doc") }
    }

    /// Bottom bar while selecting commits: cherry-pick them in history order (oldest first).
    @ViewBuilder
    private var pickBar: some View {
        if selecting, !picked.isEmpty, let r = result {
            let ordered = r.entries.filter { picked.contains($0.hash) }.reversed().map(\.hash)
            let topCount = r.entries.prefix(while: { picked.contains($0.hash) }).count
            HStack {
                Button {
                    pending = PendingCommitOp(
                        title: "Cherry-pick \(ordered.count) commits?", button: "Cherry-pick \(ordered.count)", destructive: false,
                        message: "Applies them on top of the current branch, oldest first. A conflict aborts everything and changes nothing.",
                        op: .commit(.cherryPick(hashes: Array(ordered))))
                } label: {
                    Text("Cherry-pick \(ordered.count)").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                if let rng = pickedRange(r.entries), rng.count >= 2, rng.first != 0 {
                    Button {
                        let sel = r.entries[rng]
                        squashText = sel.reversed().map(\.subject).joined(separator: "\n")
                        squashRange = (oldest: sel.last!.hash, newest: sel.first!.hash)
                    } label: {
                        Text("Squash \(rng.count)").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                if topCount >= 2 && topCount == picked.count {
                    Button {
                        squashText = r.entries.prefix(topCount).reversed().map(\.subject).joined(separator: "\n")
                        squashCount = topCount
                    } label: {
                        Text("Squash \(topCount)").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
            }
            .padding()
            .background(.bar)
        }
    }

    /// Indices of the picked commits when they form one unbroken run in the list (newest first).
    private func pickedRange(_ entries: [GitLogEntry]) -> Range<Int>? {
        let idx = entries.indices.filter { picked.contains(entries[$0].hash) }
        guard let lo = idx.first, let hi = idx.last, hi - lo + 1 == idx.count, idx.count == picked.count else { return nil }
        return lo..<(hi + 1)
    }

    private func runOp(_ op: LogOp) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        Task {
            do {
                let r: GitOpResult
                switch op {
                case .commit(let o): r = try await handle.runGitCommitOp(sessionName: name, op: o)
                case .history(let o): r = try await handle.runGitHistoryOp(sessionName: name, op: o)
                }
                await MainActor.run {
                    opMessage = r.output.isEmpty ? (r.ok ? "Done" : "Failed (exit \(r.exitCode))") : r.output
                    opFailed = !r.ok
                    if r.ok { selecting = false; picked = [] }
                    fetchLog()
                }
            } catch {
                await MainActor.run { opMessage = error.localizedDescription; opFailed = true }
            }
        }
    }

    private func row(_ e: GitLogEntry) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(verbatim: e.subject).font(.system(size: 14, weight: .medium)).lineLimit(2)
            HStack(spacing: 6) {
                Text(verbatim: e.shortHash).font(.system(size: 11, design: .monospaced)).foregroundColor(.cyan)
                Text(verbatim: "\(e.author) · \(e.when)").font(.system(size: 11)).foregroundColor(.secondary)
            }
            if !e.refs.isEmpty {
                Text(verbatim: e.refs).font(.system(size: 10, design: .monospaced)).foregroundColor(.orange).lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private func fetchLog() {
        guard viewModel.state == .connected, let handle = viewModel.sessionHandle else {
            errorMessage = "The terminal is not connected. Reconnect and try again."
            isLoading = false
            return
        }
        isLoading = true
        errorMessage = nil
        let name = viewModel.session.sessionName
        let n = limit
        Task {
            do {
                let r = try await handle.fetchGitLog(sessionName: name, limit: n)
                await MainActor.run { result = r; isLoading = false }
            } catch {
                await MainActor.run {
                    errorMessage = "Failed to read the git log: \(error.localizedDescription)"
                    isLoading = false
                }
            }
        }
    }
}

extension GitLogEntry: @retroactive Identifiable {
    public var id: String { hash }
}


/// A commit operation waiting for the user's confirmation.
struct PendingCommitOp {
    let title: String
    let button: String
    let destructive: Bool
    let message: String
    let op: LogOp
}

enum LogOp {
    case commit(GitCommitOp)
    case history(GitHistoryOp)
}

/// The confirmation and text-entry dialogs of the log sheet.
private struct CommitDialogs: ViewModifier {
    @Binding var pending: PendingCommitOp?
    @Binding var squashCount: Int?
    @Binding var squashText: String
    @Binding var squashRange: (oldest: String, newest: String)?
    @Binding var reword: GitLogEntry?
    @Binding var rewordText: String
    let rewordIsTip: Bool
    let run: (LogOp) -> Void

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                pending?.title ?? "", isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
                titleVisibility: .visible, presenting: pending
            ) { p in
                Button(p.button, role: p.destructive ? .destructive : nil) { run(p.op) }
                Button("Cancel", role: .cancel) {}
            } message: { p in
                Text(p.message)
            }
            .alert("Squash \(squashCount ?? 0) commits", isPresented: Binding(get: { squashCount != nil }, set: { if !$0 { squashCount = nil } })) {
                TextField("Message", text: $squashText, axis: .vertical)
                Button("Squash", role: .destructive) {
                    if let n = squashCount { run(.history(.squash(count: UInt32(n), message: squashText))) }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The top commits become one. Refused if any is on a remote. The old tip is saved under refs/ssh-app/backup/.")
            }
            .alert("Squash selected commits", isPresented: Binding(get: { squashRange != nil }, set: { if !$0 { squashRange = nil } })) {
                TextField("Message", text: $squashText, axis: .vertical)
                Button("Squash", role: .destructive) {
                    if let r = squashRange { run(.history(.squashRange(oldest: r.oldest, newest: r.newest, message: squashText))) }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The selected run becomes one commit; the later commits are replayed. Refused if any is on a remote or a merge is involved. The old tip is saved under refs/ssh-app/backup/.")
            }
            .alert(rewordIsTip ? "Reword last commit" : "Reword commit", isPresented: Binding(get: { reword != nil }, set: { if !$0 { reword = nil } })) {
                TextField("Message", text: $rewordText, axis: .vertical)
                Button("Save") {
                    if rewordIsTip { run(.commit(.amendMessage(message: rewordText))) }
                    else if let e = reword { run(.history(.reword(hash: e.hash, message: rewordText))) }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(rewordIsTip
                    ? "Only the message changes. Refused when the commit is already on a remote branch."
                    : "Only the message changes; the later commits are replayed on top. Refused if it is on a remote, after a merge commit, or if the tree is dirty. The old tip is saved under refs/ssh-app/backup/.")
            }
    }
}

enum ProjectTab: String, Identifiable {
    case changes, log, branches
    var id: String { rawValue }
}

/// One sheet for the git work of a tab: changes (stage, commit, push), log and branches, as tabs.
public struct ProjectSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var tab: ProjectTab
    let onChanged: () -> Void

    init(viewModel: TerminalSessionViewModel, tab: ProjectTab, onChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self._tab = State(initialValue: tab)
        self.onChanged = onChanged
    }

    public var body: some View {
        TabView(selection: $tab) {
            GitChangesSheetView(viewModel: viewModel, onChanged: onChanged)
                .tabItem { Label("Changes", systemImage: "square.and.arrow.up") }
                .tag(ProjectTab.changes)
            GitLogSheetView(viewModel: viewModel)
                .tabItem { Label("Log", systemImage: "clock.arrow.circlepath") }
                .tag(ProjectTab.log)
            GitBranchesSheetView(viewModel: viewModel, onChanged: onChanged)
                .tabItem { Label("Branches", systemImage: "arrow.triangle.branch") }
                .tag(ProjectTab.branches)
        }
    }
}
