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
    @State private var context: PaneContext? = nil

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
                    Button(action: fetchLog) { Image(systemName: "arrow.clockwise") }.disabled(isLoading)
                }
            }
            .sheet(item: $selected) { entry in
                DiffViewerSheetView(viewModel: viewModel, commit: entry)
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
                    ForEach(r.entries, id: \.hash) { e in
                        Button { selected = e } label: { row(e) }.buttonStyle(.plain)
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
