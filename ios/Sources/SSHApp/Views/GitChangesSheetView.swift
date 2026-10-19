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
                        }
                    } header: {
                        HStack { Text("Staged"); Spacer(); Button("Unstage all") { run(.unstageAll) }.font(.caption) }
                    }
                }
                Section {
                    if unstaged.isEmpty { Text("No unstaged changes").foregroundColor(.secondary) }
                    ForEach(unstaged, id: \.path) { e in
                        fileRow(e, state: e.worktreeState, checked: false) { run(.stage(paths: [e.path])) }
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
        .buttonStyle(.bordered)
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
