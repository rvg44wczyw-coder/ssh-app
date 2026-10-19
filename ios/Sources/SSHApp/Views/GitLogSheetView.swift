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
    @State private var context: PaneContext? = nil
    // Commit operations (feature 28, G3).
    @State private var selecting = false
    @State private var picked: Set<String> = []
    @State private var pending: PendingCommitOp?
    @State private var reword: GitLogEntry?
    @State private var rewordText = ""
    @State private var opMessage: String?
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
            .sheet(isPresented: $showWorkingTree) {
                DiffViewerSheetView(viewModel: viewModel)
            }
            .safeAreaInset(edge: .bottom) { pickBar }
            .confirmationDialog(
                pending?.title ?? "", isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
                titleVisibility: .visible, presenting: pending
            ) { p in
                Button(p.button, role: p.destructive ? .destructive : nil) { runOp(p.op) }
                Button("Cancel", role: .cancel) {}
            } message: { p in
                Text(p.message)
            }
            .alert("Reword last commit", isPresented: Binding(get: { reword != nil }, set: { if !$0 { reword = nil } })) {
                TextField("Message", text: $rewordText, axis: .vertical)
                Button("Save") { runOp(.amendMessage(message: rewordText)) }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Only the message changes. Refused when the commit is already on a remote branch.")
            }
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
                op: .revert(hash: e.hash))
        } label: { Label("Revert", systemImage: "arrow.uturn.backward") }
        Button {
            pending = PendingCommitOp(
                title: "Cherry-pick \(e.shortHash)?", button: "Cherry-pick", destructive: false,
                message: "Applies \"\(e.subject)\" on top of the current branch. A conflict aborts it and changes nothing.",
                op: .cherryPick(hashes: [e.hash]))
        } label: { Label("Cherry-pick here", systemImage: "arrow.down.to.line") }
        if !isTip {
            Button {
                pending = PendingCommitOp(
                    title: "Move the branch to \(e.shortHash)?", button: "Reset (keep changes staged)", destructive: true,
                    message: "The newer commits leave the branch but their changes stay staged. The old tip is saved under refs/ssh-app/backup/.",
                    op: .reset(hash: e.hash, mode: .soft))
            } label: { Label("Reset to here (soft)", systemImage: "arrow.counterclockwise") }
            Button {
                pending = PendingCommitOp(
                    title: "Move the branch to \(e.shortHash)?", button: "Reset (unstage)", destructive: true,
                    message: "The newer commits leave the branch; their changes stay in your files, unstaged. The old tip is saved under refs/ssh-app/backup/.",
                    op: .reset(hash: e.hash, mode: .mixed))
            } label: { Label("Reset to here (mixed)", systemImage: "arrow.counterclockwise") }
        } else {
            Button {
                rewordText = e.subject
                reword = e
            } label: { Label("Reword message", systemImage: "pencil") }
        }
        Button { UIPasteboard.general.string = e.hash } label: { Label("Copy hash", systemImage: "doc.on.doc") }
    }

    /// Bottom bar while selecting commits: cherry-pick them in history order (oldest first).
    @ViewBuilder
    private var pickBar: some View {
        if selecting, !picked.isEmpty, let r = result {
            let ordered = r.entries.filter { picked.contains($0.hash) }.reversed().map(\.hash)
            Button {
                pending = PendingCommitOp(
                    title: "Cherry-pick \(ordered.count) commits?", button: "Cherry-pick \(ordered.count)", destructive: false,
                    message: "Applies them on top of the current branch, oldest first. A conflict aborts everything and changes nothing.",
                    op: .cherryPick(hashes: Array(ordered)))
            } label: {
                Text("Cherry-pick \(ordered.count)").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .padding()
            .background(.bar)
        }
    }

    private func runOp(_ op: GitCommitOp) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        Task {
            do {
                let r = try await handle.runGitCommitOp(sessionName: name, op: op)
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

extension GitLogEntry: Identifiable {
    public var id: String { hash }
}


/// A commit operation waiting for the user's confirmation.
struct PendingCommitOp {
    let title: String
    let button: String
    let destructive: Bool
    let message: String
    let op: GitCommitOp
}
