import SwiftUI
import SshCoreBridge

extension Notification.Name {
    /// Posted by the worktrees sheet: open a new tab in a worktree folder (userInfo: title, path).
    static let openWorktreeTab = Notification.Name("ssh-app.open-worktree-tab")
}

/// Worktrees of the tab's repository (feature 28, G2): list, add in a sibling folder, remove.
/// Opening one either `cd`s this tab there or starts a new tab in that folder.
public struct GitWorktreesSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel

    @State private var result: GitWorktreesResult?
    @State private var branches: [GitBranch] = []
    @State private var isLoading = true
    @State private var message: String?
    @State private var messageIsError = false
    @State private var showAdd = false
    @State private var newBranch = ""
    @State private var pendingRemove: GitWorktree?
    @State private var forceRemove: GitWorktree?

    public init(viewModel: TerminalSessionViewModel) { self.viewModel = viewModel }

    public var body: some View {
        NavigationStack {
            content
                .navigationTitle("Worktrees")
                #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                    ToolbarItem(placement: .primaryAction) {
                        HStack {
                            Button { newBranch = ""; showAdd = true } label: { Image(systemName: "plus") }
                                .disabled(result?.status != .ok)
                            Button(action: load) { Image(systemName: "arrow.clockwise") }.disabled(isLoading)
                        }
                    }
                }
                .onAppear(perform: load)
                .sheet(isPresented: $showAdd) { addSheet }
                .confirmationDialog(
                    "Remove worktree", isPresented: Binding(get: { pendingRemove != nil }, set: { if !$0 { pendingRemove = nil } }),
                    titleVisibility: .visible, presenting: pendingRemove
                ) { w in
                    Button("Remove \(folderName(w))", role: .destructive) { remove(w, force: false) }
                    Button("Cancel", role: .cancel) {}
                } message: { w in
                    Text("Deletes the folder \(w.path). Git refuses when it has uncommitted changes. The branch itself stays.")
                }
                .confirmationDialog(
                    "Uncommitted changes", isPresented: Binding(get: { forceRemove != nil }, set: { if !$0 { forceRemove = nil } }),
                    titleVisibility: .visible, presenting: forceRemove
                ) { w in
                    Button("Remove and lose the changes", role: .destructive) { remove(w, force: true) }
                    Button("Cancel", role: .cancel) {}
                } message: { w in
                    Text("\(w.path) has changes that are not committed. Removing it discards them for good.")
                }
        }
    }

    private func folderName(_ w: GitWorktree) -> String { (w.path as NSString).lastPathComponent }

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
                Section {
                    ForEach(r.worktrees, id: \.path) { row($0) }
                } footer: {
                    Text("New worktrees are created next to the repository as <repo>-<branch>.")
                }
            }
        } else {
            VStack(spacing: 8) {
                Image(systemName: "square.on.square").font(.largeTitle).foregroundColor(.secondary)
                Text(result?.status == .notARepo ? "This tab is not in a git repository" : (message ?? "Could not read worktrees"))
                    .foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func row(_ w: GitWorktree) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: w.isMain ? "house" : "square.on.square")
                    .foregroundColor(w.isCurrent ? .green : .secondary)
                Text(w.branch ?? (w.detached ? "detached \(w.headShort)" : w.headShort))
                    .font(.system(size: 14, weight: w.isCurrent ? .bold : .regular, design: .monospaced))
                if w.isCurrent { Text("this tab").font(.caption2).foregroundColor(.green) }
                if w.locked { Image(systemName: "lock.fill").font(.caption).foregroundColor(.orange) }
                if w.prunable { Text("missing").font(.caption2).foregroundColor(.red) }
            }
            Text(w.path).font(.system(size: 11, design: .monospaced)).foregroundColor(.secondary)
                .lineLimit(1).truncationMode(.head)
            if !w.isCurrent && !w.prunable {
                HStack(spacing: 10) {
                    Button("cd here") {
                        viewModel.changeDirectory(to: w.path)
                        dismiss()
                    }
                    Button("Open in new tab") {
                        NotificationCenter.default.post(
                            name: .openWorktreeTab, object: nil,
                            userInfo: ["title": w.branch ?? folderName(w), "path": w.path])
                        dismiss()
                    }
                }
                .font(.caption.bold())
                .buttonStyle(.glass)
            }
        }
        .swipeActions(edge: .trailing) {
            if !w.isMain && !w.isCurrent {
                Button(role: .destructive) { pendingRemove = w } label: { Label("Remove", systemImage: "trash") }
            }
        }
    }

    private var addSheet: some View {
        NavigationStack {
            Form {
                Section("New branch in a new worktree") {
                    TextField("feature/name", text: $newBranch)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Create from the current commit") { add(branch: newBranch, isNew: true) }
                        .disabled(newBranch.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                if !free.isEmpty {
                    Section("Existing branch (not checked out anywhere)") {
                        ForEach(free, id: \.name) { b in
                            Button(b.name) { add(branch: b.name, isNew: false) }
                        }
                    }
                }
            }
            .navigationTitle("Add worktree")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { showAdd = false } } }
        }
        .presentationDetents([.medium, .large])
    }

    /// Local branches no worktree has checked out.
    private var free: [GitBranch] {
        let used = Set(result?.worktrees.compactMap(\.branch) ?? [])
        return branches.filter { !$0.isRemote && !used.contains($0.name) }
    }

    private func load() {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        isLoading = true
        Task {
            do {
                let r = try await handle.fetchGitWorktrees(sessionName: name)
                let b = try? await handle.fetchGitBranches(sessionName: name)
                await MainActor.run { result = r; branches = b?.branches ?? []; isLoading = false }
            } catch {
                await MainActor.run { message = error.localizedDescription; isLoading = false }
            }
        }
    }

    private func run(_ op: GitWorktreeOp, onFailure: ((GitOpResult) -> Void)? = nil) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        Task {
            do {
                let r = try await handle.runGitWorktreeOp(sessionName: name, op: op)
                await MainActor.run {
                    message = r.output.isEmpty ? (r.ok ? "Done" : "Failed (exit \(r.exitCode))") : r.output
                    messageIsError = !r.ok
                    if !r.ok { onFailure?(r) }
                    load()
                }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true }
            }
        }
    }

    private func add(branch: String, isNew: Bool) {
        showAdd = false
        run(.add(branch: branch.trimmingCharacters(in: .whitespaces), newBranch: isNew, from: nil))
    }

    private func remove(_ w: GitWorktree, force: Bool) {
        run(.remove(path: w.path, force: force)) { r in
            if !force, r.output.lowercased().contains("contains modified or untracked") || r.output.lowercased().contains("use --force") {
                forceRemove = w
            }
        }
    }
}
