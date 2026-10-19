import SwiftUI
import SshCoreBridge

/// Branches of the repository the tab works in: switch, create, delete (feature 28, G1). Every
/// change is confirmed with its exact effect; deleting saves the tip under `refs/ssh-app/backup/`.
public struct GitBranchesSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel
    /// Called after a successful change so the caller can refresh what it shows (file colours).
    var onChanged: () -> Void = {}

    @State private var result: GitBranchesResult?
    @State private var isLoading = true
    @State private var message: String?
    @State private var messageIsError = false
    @State private var pendingSwitch: GitBranch?
    @State private var pendingDelete: GitBranch?
    @State private var forceDelete: GitBranch?
    @State private var showCreate = false
    @State private var newName = ""
    @State private var createFrom: String?
    @State private var showWorktrees = false

    public init(viewModel: TerminalSessionViewModel, onChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.onChanged = onChanged
    }

    private var local: [GitBranch] { result?.branches.filter { !$0.isRemote } ?? [] }
    private var remote: [GitBranch] { result?.branches.filter(\.isRemote) ?? [] }

    public var body: some View {
        NavigationStack {
            content
                .navigationTitle("Branches")
                #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                    ToolbarItem(placement: .primaryAction) {
                        HStack {
                            Button { createFrom = nil; newName = ""; showCreate = true } label: { Image(systemName: "plus") }
                                .disabled(result?.status != .ok)
                            Button { showWorktrees = true } label: { Image(systemName: "square.on.square") }
                                .accessibilityLabel("Worktrees")
                                .disabled(result?.status != .ok)
                            Button(action: load) { Image(systemName: "arrow.clockwise") }.disabled(isLoading)
                        }
                    }
                }
                .onAppear(perform: load)
                .sheet(isPresented: $showWorktrees) { GitWorktreesSheetView(viewModel: viewModel) }
                .alert("New branch", isPresented: $showCreate) {
                    TextField("feature/name", text: $newName)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Create and switch") { create(switchTo: true) }
                    Button("Create") { create(switchTo: false) }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("Starts at \(createFrom ?? result?.current ?? "the current commit").")
                }
                .confirmationDialog(
                    "Switch branch", isPresented: Binding(get: { pendingSwitch != nil }, set: { if !$0 { pendingSwitch = nil } }),
                    titleVisibility: .visible, presenting: pendingSwitch
                ) { b in
                    Button(b.isRemote ? "Create \"\(localName(b))\" tracking \(b.name)" : "Switch to \(b.name)") { switchTo(b) }
                    Button("Cancel", role: .cancel) {}
                } message: { b in
                    Text(result?.dirty == true
                        ? "You have uncommitted changes. Git carries them over, or refuses if they conflict."
                        : "Checks out \(b.name) in this tab's repository.")
                }
                .confirmationDialog(
                    "Delete branch", isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
                    titleVisibility: .visible, presenting: pendingDelete
                ) { b in
                    Button("Delete \(b.name)", role: .destructive) { delete(b, force: false) }
                    Button("Cancel", role: .cancel) {}
                } message: { b in
                    Text("Only if it is fully merged. Its tip is saved under refs/ssh-app/backup/ (\(b.shortHash)).")
                }
                .confirmationDialog(
                    "Not fully merged", isPresented: Binding(get: { forceDelete != nil }, set: { if !$0 { forceDelete = nil } }),
                    titleVisibility: .visible, presenting: forceDelete
                ) { b in
                    Button("Delete anyway", role: .destructive) { delete(b, force: true) }
                    Button("Cancel", role: .cancel) {}
                } message: { b in
                    Text("\(b.name) has commits that are on no other branch. They stay reachable through the backup ref refs/ssh-app/backup/ until you remove it.")
                }
        }
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
                if r.detached {
                    Section { Label("Detached HEAD", systemImage: "exclamationmark.triangle").foregroundColor(.orange) }
                }
                Section("Local") {
                    ForEach(local, id: \.name) { row($0) }
                }
                if !remote.isEmpty {
                    Section("Remote") { ForEach(remote, id: \.name) { row($0) } }
                }
            }
        } else {
            VStack(spacing: 8) {
                Image(systemName: "arrow.triangle.branch").font(.largeTitle).foregroundColor(.secondary)
                Text(statusText).foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var statusText: String {
        switch result?.status {
        case .notARepo: return "This tab is not in a git repository"
        case .noSession: return "tmux session not found"
        default: return message ?? "Could not read branches"
        }
    }

    private func localName(_ b: GitBranch) -> String {
        b.isRemote ? String(b.name.drop(while: { $0 != "/" }).dropFirst()) : b.name
    }

    private func row(_ b: GitBranch) -> some View {
        Button {
            if !b.isCurrent { pendingSwitch = b }
        } label: {
            HStack(spacing: 8) {
                Image(systemName: b.isCurrent ? "checkmark.circle.fill" : (b.isRemote ? "cloud" : "arrow.triangle.branch"))
                    .foregroundColor(b.isCurrent ? .green : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(b.name).font(.system(size: 14, weight: b.isCurrent ? .bold : .regular, design: .monospaced))
                    Text("\(b.shortHash)  \(b.subject)").font(.caption).foregroundColor(.secondary).lineLimit(1)
                }
                Spacer()
                if b.ahead > 0 { Text("↑\(b.ahead)").font(.caption.bold()).foregroundColor(.green) }
                if b.behind > 0 { Text("↓\(b.behind)").font(.caption.bold()).foregroundColor(.orange) }
            }
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button { createFrom = b.name; newName = ""; showCreate = true } label: {
                Label("New branch from here…", systemImage: "plus")
            }
        }
        .swipeActions(edge: .trailing) {
            if !b.isRemote && !b.isCurrent {
                Button(role: .destructive) { pendingDelete = b } label: { Label("Delete", systemImage: "trash") }
            }
        }
    }

    private func load() {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        isLoading = true
        Task {
            do {
                let r = try await handle.fetchGitBranches(sessionName: name)
                await MainActor.run { result = r; isLoading = false }
            } catch {
                await MainActor.run { message = error.localizedDescription; isLoading = false }
            }
        }
    }

    private func run(_ op: GitBranchOp, onFailure: ((GitOpResult) -> Void)? = nil) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        Task {
            do {
                let r = try await handle.runGitBranchOp(sessionName: name, op: op)
                await MainActor.run {
                    message = r.output.isEmpty ? (r.ok ? "Done" : "Failed (exit \(r.exitCode))") : r.output
                    messageIsError = !r.ok
                    if r.ok { onChanged() } else { onFailure?(r) }
                    load()
                }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true }
            }
        }
    }

    private func switchTo(_ b: GitBranch) {
        run(b.isRemote ? .switchTracking(name: b.name) : .switch(name: b.name))
    }

    private func create(switchTo: Bool) {
        let name = newName.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        run(.create(name: name, from: createFrom, switchTo: switchTo))
    }

    private func delete(_ b: GitBranch, force: Bool) {
        run(.delete(name: b.name, force: force)) { r in
            if !force, r.output.contains("not fully merged") { forceDelete = b }
        }
    }
}
