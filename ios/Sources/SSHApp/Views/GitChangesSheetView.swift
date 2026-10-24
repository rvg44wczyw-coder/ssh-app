import SwiftUI
import SshCoreBridge

/// Stage, commit, fetch / pull / push for the tab's repository (feature 28, G5). Nothing here
/// discards changes or forces a push; for that the app points to the terminal.
public struct GitChangesSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel
    var onChanged: () -> Void = {}

    @State private var result: GitChangesResult?
    @State private var isLoading = true
    @State private var busy = false
    @State private var message: String?
    @State private var messageIsError = false
    @State private var commitText = ""
    @State private var confirmPush = false
    @State private var hunkTarget: HunkTarget?

    public init(viewModel: TerminalSessionViewModel, onChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.onChanged = onChanged
    }

    private var staged: [GitChangeEntry] { result?.entries.filter { $0.indexState != nil && $0.indexState != .untracked } ?? [] }
    private var unstaged: [GitChangeEntry] { result?.entries.filter { $0.worktreeState != nil } ?? [] }

    public var body: some View {
        NavigationStack {
            content
                .navigationTitle("Changes")
                #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                    ToolbarItem(placement: .primaryAction) {
                        if busy { ProgressView() } else { Button(action: load) { Image(systemName: "arrow.clockwise") } }
                    }
                }
                .safeAreaInset(edge: .bottom) { remoteBar }
                .onAppear(perform: load)
                .confirmationDialog("Push", isPresented: $confirmPush, titleVisibility: .visible) {
                    Button("Push") { run(.push) }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text(pushText)
                }
                .sheet(item: $hunkTarget, onDismiss: load) { target in
                    GitHunksSheetView(viewModel: viewModel, path: target.path, staged: target.staged, onChanged: onChanged)
                }
        }
    }

    private var pushText: String {
        guard let r = result else { return "" }
        let target = r.upstream ?? "a new upstream branch"
        return "\(r.ahead) commit\(r.ahead == 1 ? "" : "s") from \(r.branch ?? "HEAD") to \(target). Never forced: if the remote has newer commits it is refused."
    }

    @ViewBuilder
    private var content: some View {
        if isLoading && result == nil {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let r = result, r.status == .ok {
            List {
                if let message {
                    Section {
                        Text(message).font(.system(size: 12, design: .monospaced))
                            .foregroundColor(messageIsError ? .red : .secondary)
                    }
                }
                Section("Commit") {
                    TextField("Message", text: $commitText, axis: .vertical).lineLimit(1...6)
                    Button("Commit \(staged.count) staged file\(staged.count == 1 ? "" : "s")") {
                        run(.commit(message: commitText)) { commitText = "" }
                    }
                    .disabled(staged.isEmpty || commitText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy)
                }
                if !staged.isEmpty {
                    Section {
                        ForEach(staged, id: \.path) { e in
                            fileRow(e, state: e.indexState, checked: true) { run(.unstage(paths: [e.path])) }
                                .swipeActions { hunkAction(e, staged: true) }
                        }
                    } header: {
                        HStack { Text("Staged"); Spacer(); Button("Unstage all") { run(.unstageAll) }.font(.caption) }
                    }
                }
                Section {
                    if unstaged.isEmpty { Text("No unstaged changes").foregroundColor(.secondary) }
                    ForEach(unstaged, id: \.path) { e in
                        fileRow(e, state: e.worktreeState, checked: false) { run(.stage(paths: [e.path])) }
                            .swipeActions { hunkAction(e, staged: false) }
                    }
                } header: {
                    HStack { Text("Not staged"); Spacer(); if !unstaged.isEmpty { Button("Stage all") { run(.stageAll) }.font(.caption) } }
                } footer: {
                    Text("To discard changes use the terminal; the app never throws work away.")
                }
            }
        } else {
            VStack(spacing: 8) {
                Image(systemName: "tray").font(.largeTitle).foregroundColor(.secondary)
                Text(result?.status == .notARepo ? "This tab is not in a git repository" : (message ?? "Could not read the repository"))
                    .foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// Single hunks make sense for text changes of tracked files (not new, deleted or renamed ones).
    @ViewBuilder
    private func hunkAction(_ e: GitChangeEntry, staged: Bool) -> some View {
        let state = staged ? e.indexState : e.worktreeState
        if state == .modified {
            Button { hunkTarget = HunkTarget(path: e.path, staged: staged) } label: {
                Label("Hunks", systemImage: "list.bullet.indent")
            }
            .tint(.indigo)
        }
    }

    private func fileRow(_ e: GitChangeEntry, state: GitFileState?, checked: Bool, toggle: @escaping () -> Void) -> some View {
        Button(action: toggle) {
            HStack(spacing: 10) {
                Image(systemName: checked ? "checkmark.circle.fill" : "circle")
                    .foregroundColor(checked ? .accentColor : .secondary)
                if let state {
                    GitStateBadge(state: state)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text((e.path as NSString).lastPathComponent).font(.system(size: 14, design: .monospaced))
                    let dir = (e.path as NSString).deletingLastPathComponent
                    if !dir.isEmpty { Text(dir).font(.caption2).foregroundColor(.secondary) }
                }
                Spacer()
            }
        }
        .buttonStyle(.plain)
        .disabled(busy)
    }

    private var remoteBar: some View {
        HStack(spacing: 10) {
            Button { run(.fetch) } label: { Label("Fetch", systemImage: "arrow.triangle.2.circlepath").frame(maxWidth: .infinity) }
            Button { run(.pull) } label: {
                Label(result?.behind ?? 0 > 0 ? "Pull ↓\(result?.behind ?? 0)" : "Pull", systemImage: "arrow.down.to.line").frame(maxWidth: .infinity)
            }
            Button { confirmPush = true } label: {
                Label(result?.ahead ?? 0 > 0 ? "Push ↑\(result?.ahead ?? 0)" : "Push", systemImage: "arrow.up.to.line").frame(maxWidth: .infinity)
            }
        }
        .buttonStyle(.glass)
        .disabled(busy || result?.status != .ok)
        .padding()
        .background(.bar)
    }

    private func load() {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        isLoading = true
        Task {
            do {
                let r = try await handle.fetchGitChanges(sessionName: name)
                await MainActor.run { result = r; isLoading = false }
            } catch {
                await MainActor.run { message = error.localizedDescription; isLoading = false }
            }
        }
    }

    private func run(_ op: GitIndexOp, onSuccess: (() -> Void)? = nil) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        busy = true
        Task {
            do {
                let r = try await handle.runGitIndexOp(sessionName: name, op: op)
                await MainActor.run {
                    message = r.output.isEmpty ? (r.ok ? "Done" : "Failed (exit \(r.exitCode))") : r.output
                    messageIsError = !r.ok
                    if r.ok { onSuccess?(); onChanged() }
                    busy = false
                    load()
                }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true; busy = false }
            }
        }
    }
}

struct HunkTarget: Identifiable {
    let path: String
    let staged: Bool
    var id: String { "\(staged ? "s" : "w"):\(path)" }
}

/// The hunks of one file's diff with a button per hunk to stage it (or take it out of the index
/// again). The working tree is never touched; a hunk that no longer matches is refused by git.
struct GitHunksSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel
    let path: String
    let staged: Bool
    var onChanged: () -> Void

    @State private var hunks: GitFileHunks?
    @State private var loading = true
    @State private var busy = false
    @State private var message: String?
    @State private var messageIsError = false

    var body: some View {
        NavigationStack {
            List {
                if let message {
                    Text(message).font(.caption).foregroundColor(messageIsError ? .red : .secondary)
                }
                if loading && hunks == nil {
                    ProgressView()
                } else if let hunks {
                    if hunks.binary { Text("Binary file: stage the whole file.").foregroundColor(.secondary) }
                    if hunks.hunks.isEmpty && !hunks.binary { Text("No hunks left.").foregroundColor(.secondary) }
                    if hunks.truncated { Text("The diff is long; only the first hunks are shown.").font(.caption).foregroundColor(.orange) }
                    ForEach(Array(hunks.hunks.enumerated()), id: \.offset) { _, hunk in
                        Section {
                            ForEach(Array(hunk.lines.enumerated()), id: \.offset) { _, line in
                                Text(line)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundColor(color(line))
                                    .listRowBackground(background(line))
                            }
                            Button(staged ? "Unstage this hunk" : "Stage this hunk") { apply(hunk) }
                                .disabled(busy || !hunk.stageable)
                            if !hunk.stageable {
                                Text("Has unusual characters: stage the whole file instead.").font(.caption2).foregroundColor(.secondary)
                            }
                        } header: {
                            Text("\(hunk.header)  +\(hunk.added) −\(hunk.removed)").font(.system(size: 10, design: .monospaced))
                        }
                    }
                }
            }
            .navigationTitle((path as NSString).lastPathComponent)
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } } }
            .onAppear(perform: load)
        }
    }

    private func color(_ line: String) -> Color {
        line.hasPrefix("+") ? .green : (line.hasPrefix("-") ? .red : .primary)
    }

    private func background(_ line: String) -> Color? {
        line.hasPrefix("+") ? Color.green.opacity(0.10) : (line.hasPrefix("-") ? Color.red.opacity(0.10) : nil)
    }

    private func load() {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        loading = true
        Task {
            do {
                let r = try await handle.fetchGitFileHunks(sessionName: name, path: path, staged: staged)
                await MainActor.run { hunks = r; loading = false }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true; loading = false }
            }
        }
    }

    private func apply(_ hunk: GitHunk) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        busy = true
        Task {
            do {
                let r = try await handle.applyGitHunks(sessionName: name, path: path, hunks: [hunk], unstage: staged)
                await MainActor.run {
                    message = r.ok ? nil : (r.output.isEmpty ? "Failed (exit \(r.exitCode))" : r.output)
                    messageIsError = !r.ok
                    if r.ok { onChanged() }
                    busy = false
                    load()
                }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true; busy = false }
            }
        }
    }
}
