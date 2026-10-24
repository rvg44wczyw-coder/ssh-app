import SwiftUI
import SshCoreBridge

/// Stashes of the tab's repository (feature 28, G4): save the working changes aside, bring them
/// back, drop one. Every action keeps a backup ref of the stash commit.
public struct GitStashesSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel
    var onChanged: () -> Void = {}

    @State private var stashes: [GitStash] = []
    @State private var isLoading = true
    @State private var message: String?
    @State private var messageIsError = false
    @State private var showPush = false
    @State private var pushText = ""
    @State private var includeUntracked = false
    @State private var pendingDrop: GitStash?

    public init(viewModel: TerminalSessionViewModel, onChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.onChanged = onChanged
    }

    public var body: some View {
        NavigationStack {
            List {
                if let message {
                    Section {
                        Text(message).font(.system(size: 12, design: .monospaced))
                            .foregroundColor(messageIsError ? .red : .secondary)
                    }
                }
                Section {
                    if stashes.isEmpty && !isLoading {
                        Text("No stashes").foregroundColor(.secondary)
                    }
                    ForEach(stashes, id: \.index) { s in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(s.message).font(.system(size: 14))
                            Text("stash@{\(s.index)} · \(s.when)").font(.caption).foregroundColor(.secondary)
                            HStack(spacing: 10) {
                                Button("Apply") { run(.stashApply(index: UInt32(s.index))) }
                                Button("Pop") { run(.stashPop(index: UInt32(s.index))) }
                            }
                            .font(.caption.bold())
                            .buttonStyle(.glass)
                        }
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) { pendingDrop = s } label: { Label("Drop", systemImage: "trash") }
                        }
                    }
                } footer: {
                    Text("Apply and Pop need a clean tree; if the stash does not apply cleanly it stays in the list and your files are left as they were.")
                }
            }
            .navigationTitle("Stashes")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button { pushText = ""; showPush = true } label: { Image(systemName: "plus") }
                }
            }
            .onAppear(perform: load)
            .alert("Stash changes", isPresented: $showPush) {
                TextField("Message", text: $pushText)
                Button("Stash tracked changes") { push() }
                Button("Include untracked files") { includeUntracked = true; push() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Puts the uncommitted changes aside and restores a clean tree.")
            }
            .confirmationDialog(
                "Drop stash", isPresented: Binding(get: { pendingDrop != nil }, set: { if !$0 { pendingDrop = nil } }),
                titleVisibility: .visible, presenting: pendingDrop
            ) { s in
                Button("Drop stash@{\(s.index)}", role: .destructive) { run(.stashDrop(index: UInt32(s.index))) }
                Button("Cancel", role: .cancel) {}
            } message: { s in
                Text("\"\(s.message)\" is removed from the list. Its commit is saved under refs/ssh-app/backup/.")
            }
        }
    }

    private func push() {
        let text = pushText.trimmingCharacters(in: .whitespaces)
        let untracked = includeUntracked
        includeUntracked = false
        run(.stashPush(message: text.isEmpty ? "stash from phone" : text, includeUntracked: untracked))
    }

    private func load() {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        isLoading = true
        Task {
            let r = try? await handle.fetchGitStashes(sessionName: name)
            await MainActor.run { stashes = r?.stashes ?? []; isLoading = false }
        }
    }

    private func run(_ op: GitHistoryOp) {
        guard let handle = viewModel.sessionHandle else { return }
        let name = viewModel.session.sessionName
        Task {
            do {
                let r = try await handle.runGitHistoryOp(sessionName: name, op: op)
                await MainActor.run {
                    message = r.output.isEmpty ? (r.ok ? "Done" : "Failed (exit \(r.exitCode))") : r.output
                    messageIsError = !r.ok
                    if r.ok { onChanged() }
                    load()
                }
            } catch {
                await MainActor.run { message = error.localizedDescription; messageIsError = true }
            }
        }
    }
}
