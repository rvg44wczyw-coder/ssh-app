import SwiftUI
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#endif

/// Patch of one commit of the repository the active terminal tab is working in (not the SSH login
/// directory: an exec channel starts in $HOME), or, with no commit, the uncommitted changes
/// (`git diff HEAD`, untracked files listed).
public struct DiffViewerSheetView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var viewModel: TerminalSessionViewModel
    let commit: GitLogEntry?

    @State private var result: GitDiffResult? = nil
    @State private var isLoading: Bool = true
    @State private var errorMessage: String? = nil
    @State private var colours: [Int: AttributedString] = [:]

    public init(viewModel: TerminalSessionViewModel, commit: GitLogEntry? = nil) {
        self.viewModel = viewModel
        self.commit = commit
    }

    private var diffText: String { result?.diff ?? "" }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if isLoading {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text(commit == nil ? "Fetching the changes from the host..." : "Fetching the commit from the host...")
                            .font(.system(size: 13))
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let error = errorMessage {
                    VStack(spacing: 12) {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.system(size: 32))
                            .foregroundColor(.orange)
                        Text(error)
                            .font(.system(size: 13))
                            .foregroundColor(.secondary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal)
                        Button("Retry") {
                            fetchDiff()
                        }
                        .buttonStyle(.glass)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let r = result, r.status != .ok {
                    statusView(r)
                } else if diffText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    VStack(spacing: 12) {
                        Image(systemName: "checkmark.circle")
                            .font(.system(size: 36))
                            .foregroundColor(.green)
                        Text(commit == nil ? "No uncommitted changes" : "This commit has no changes")
                            .font(.system(size: 14, weight: .semibold))
                        if let root = result?.repoRoot {
                            Text(verbatim: root)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                        untrackedList
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView([.horizontal, .vertical]) {
                        LazyVStack(alignment: .leading, spacing: 1) {
                            headerView
                            ForEach(Array(diffText.components(separatedBy: "\n").enumerated()), id: \.offset) { i, line in
                                diffLineView(line, code: colours[i])
                            }
                        }
                        .padding(8)
                    }
                    .background(Color.black)
                }
            }
            .navigationTitle(commit?.shortHash ?? "Uncommitted")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button(action: fetchDiff) {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(isLoading)
                }
            }
            .onAppear {
                fetchDiff()
            }
            .task(id: diffText.count) {
                // The diff sits on a black background in both appearances: dark theme always.
                colours = await DiffHighlighter.highlight(diff: diffText, dark: true)
            }
        }
    }

    private func diffLineView(_ line: String, code: AttributedString?) -> some View {
        let (bgColor, fgColor) = lineColors(for: line)
        // With syntax colours the marker keeps the line's colour and the code is coloured by language.
        let text: Text = {
            guard let code, let marker = line.first else { return Text(line.isEmpty ? " " : line) }
            return Text(String(marker)).foregroundColor(fgColor) + Text(code)
        }()
        return text
            .font(.system(size: 11, design: .monospaced))
            .foregroundColor(code == nil ? fgColor : .white)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(bgColor)
    }

    private func lineColors(for line: String) -> (Color, Color) {
        if line.hasPrefix("+++") || line.hasPrefix("---") {
            return (Color.gray.opacity(0.2), .primary)
        } else if line.hasPrefix("+") {
            return (Color.green.opacity(0.18), .green)
        } else if line.hasPrefix("-") {
            return (Color.red.opacity(0.18), .red)
        } else if line.hasPrefix("@@") {
            return (Color.cyan.opacity(0.15), .cyan)
        } else if line.hasPrefix("diff --git") {
            return (Color.yellow.opacity(0.2), .yellow)
        } else {
            return (Color.clear, .primary)
        }
    }

    @ViewBuilder
    private var headerView: some View {
        if let r = result {
            VStack(alignment: .leading, spacing: 2) {
                if let root = r.repoRoot {
                    Text(verbatim: "\(root)\(r.branch.map { "  (\($0))" } ?? "")")
                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                        .foregroundColor(.cyan)
                }
                if r.truncated {
                    Text("Patch truncated to the first 1 MB")
                        .font(.system(size: 11)).foregroundColor(.orange)
                }
                if !r.untracked.isEmpty {
                    Text(verbatim: "Untracked (not in the diff): " + r.untracked.joined(separator: ", "))
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)
                        .lineLimit(3)
                }
            }
            .padding(.bottom, 6)
        }
    }

    @ViewBuilder
    private var untrackedList: some View {
        if let r = result, !r.untracked.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                Text("Untracked files (not in a diff):")
                    .font(.system(size: 12, weight: .semibold))
                ForEach(r.untracked, id: \.self) { f in
                    Text(verbatim: f).font(.system(size: 11, design: .monospaced)).foregroundColor(.secondary)
                }
            }
            .padding(.top, 8)
        }
    }

    @ViewBuilder
    private func statusView(_ r: GitDiffResult) -> some View {
        VStack(spacing: 12) {
            Image(systemName: "folder.badge.questionmark")
                .font(.system(size: 34))
                .foregroundColor(.orange)
            switch r.status {
            case .notARepo:
                Text("Not a git repository")
                    .font(.system(size: 14, weight: .semibold))
                if let dir = r.directory {
                    Text(verbatim: "This terminal is in \(dir). `cd` into a repository (or ask the agent to) and refresh.")
                        .font(.system(size: 12)).foregroundColor(.secondary)
                        .multilineTextAlignment(.center).padding(.horizontal)
                }
            case .noSession:
                Text("Terminal session not found on the host")
                    .font(.system(size: 14, weight: .semibold))
                Text("Open the tab's tmux session first, then refresh.")
                    .font(.system(size: 12)).foregroundColor(.secondary)
            default:
                Text("Unexpected answer from the host")
                    .font(.system(size: 14, weight: .semibold))
            }
            Button("Refresh") { fetchDiff() }.buttonStyle(.glass)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func fetchDiff() {
        guard viewModel.state == .connected, let handle = viewModel.sessionHandle else {
            self.errorMessage = "The terminal is not connected. Reconnect and try again."
            self.isLoading = false
            return
        }
        isLoading = true
        errorMessage = nil
        let name = viewModel.session.sessionName
        Task {
            do {
                let r: GitDiffResult
                if let commit {
                    r = try await handle.fetchGitShow(sessionName: name, hash: commit.hash)
                } else {
                    r = try await handle.fetchGitDiff(sessionName: name)
                }
                await MainActor.run {
                    self.result = r
                    self.isLoading = false
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = "Failed to read the \(commit == nil ? "changes" : "commit"): \(error.localizedDescription)"
                    self.isLoading = false
                }
            }
        }
    }
}
