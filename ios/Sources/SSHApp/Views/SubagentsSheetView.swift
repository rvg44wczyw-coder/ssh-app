import SshCoreBridge
import SwiftUI

/// The subagents the session has started. A tap opens one agent's own feed (read only); the feed
/// is read while its screen is open and not at all otherwise.
struct SubagentsSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var agents: [SubagentInfo] = []
    @State private var loading = true
    @State private var failed = false

    var body: some View {
        NavigationStack {
            List {
                if failed {
                    Text("The host did not answer, or this tab has no session file (Antigravity needs the host helper).").foregroundColor(.secondary)
                } else if agents.isEmpty && !loading {
                    Text("No subagents in this session yet.").foregroundColor(.secondary)
                }
                ForEach(agents, id: \.agentId) { agent in
                    NavigationLink {
                        SubagentFeedView(viewModel: viewModel, agent: agent)
                    } label: { row(agent) }
                }
            }
            .overlay { if loading && agents.isEmpty { ProgressView() } }
            .navigationTitle("Subagents")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button { Task { await load() } } label: { Image(systemName: "arrow.clockwise") }
                        .accessibilityLabel("Refresh subagents")
                }
            }
            .refreshable { await load() }
            .task { await load() }
        }
    }

    private func row(_ agent: SubagentInfo) -> some View {
        HStack(spacing: 10) {
            Image(systemName: agent.running ? "person.2.wave.2.fill" : "person.2.fill")
                .foregroundColor(agent.running ? .accentColor : .secondary)
                .accessibilityLabel(agent.running ? "Working" : "Finished")
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: agent.description.isEmpty ? agent.agentType : agent.description)
                    .font(.subheadline).lineLimit(2)
                Text(verbatim: [agent.agentType, agent.model, agent.depth > 1 ? "level \(agent.depth)" : "", agent.background ? "background" : ""]
                    .filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption).foregroundColor(.secondary).lineLimit(1)
            }
        }
    }

    private func load() async {
        loading = true
        if let list = await viewModel.fetchSubagents() {
            agents = list
            failed = false
        } else {
            failed = true
        }
        loading = false
    }
}

/// One subagent's feed, read by cursor into its own `FeedState` while this screen is open.
struct SubagentFeedView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    let agent: SubagentInfo
    @State private var feed = FeedState.newSubagent()
    @State private var cursor: FeedCursor?
    @State private var turns: [TranscriptTurn] = []
    @State private var expandedThinking: Set<String> = []
    @State private var expandedTools: Set<String> = []
    @State private var atBottom = true
    @State private var missing = false
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage("feedTextScale") private var feedTextScale: Double = 1.0
    @AppStorage(FeedPresentationStyle.storageKey) private var feedStyleRaw: String = FeedPresentationStyle.fallback.rawValue

    var body: some View {
        FeedTextView(
            turns: turns,
            expandedThinking: expandedThinking,
            expandedTools: expandedTools,
            onToggle: { toggle in
                switch toggle {
                case .thinking(let id):
                    if expandedThinking.contains(id) { expandedThinking.remove(id) } else { expandedThinking.insert(id) }
                case .resend, .cancelQueued:
                    break
                case .toolBack(let id, _):
                    expandedTools.remove(id)
                    expandedTools.remove(id + "+")
                case .tool(let id):
                    if expandedTools.contains(id) { expandedTools.remove(id) } else { expandedTools.insert(id) }
                }
            },
            atBottom: $atBottom,
            scrollToken: 0,
            findToken: 0,
            dark: colorScheme == .dark,
            highlightVersion: 0,
            textScale: CGFloat(feedTextScale),
            onScale: { feedTextScale = Double($0) },
            presentation: FeedPresentationStyle.resolved(feedStyleRaw),
            onOpenFile: { viewModel.openFileReference(path: $0, line: $1) }
        )
        .sheet(item: $viewModel.fileReference, onDismiss: { viewModel.filePreview = nil }) { target in
            FilePreviewView(viewModel: viewModel, focusLine: target.line)
        }
        .background(Color.appBackground)
        .overlay {
            if missing && turns.isEmpty {
                ContentUnavailableView("Nothing to show", systemImage: "person.2", description: Text("This subagent has no file on the host."))
            }
        }
        .navigationTitle(agent.description.isEmpty ? agent.agentType : agent.description)
        .navigationBarTitleDisplayMode(.inline)
        // Reads at once, then every few seconds while the screen is open; the task ends with it.
        .task {
            while !Task.isCancelled {
                await sync()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private func sync() async {
        guard let answer = await viewModel.readSubagent(agent.agentId, cursor: cursor) else { return }
        guard let read = answer else { missing = true; return }
        missing = false
        if read.reset { feed.reset() }
        _ = feed.applyChunk(chunk: read.text)
        cursor = read.cursor
        let now = feed.turns()
        if now != turns { turns = now }
    }
}
