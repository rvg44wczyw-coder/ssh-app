import SwiftUI
import SshCoreBridge

public struct StructuredTranscriptView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var promptText: String = ""
    @State private var atBottom = true
    @State private var unseenNew = false
    @FocusState private var inputFocused: Bool
    @State private var expandedThinkingTurns: Set<String> = []
    @State private var expandedToolCalls: Set<String> = []

    public init(viewModel: TerminalSessionViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Header Bar: Active path & Refresh action
            HStack(spacing: 8) {
                Image(systemName: "doc.text.magnifyingglass")
                    .font(.system(size: 11))
                    .foregroundColor(.appAccent)

                if let path = viewModel.activeTranscriptPath {
                    Text(path)
                        .font(.system(size: 10, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundColor(.secondary)
                } else {
                    Text("Auto-detecting transcript...")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }

                Spacer()

                if viewModel.isTranscriptLoading {
                    ProgressView()
                        .scaleEffect(0.7)
                } else {
                    Button(action: {
                        viewModel.loadTranscript()
                    }) {
                        HStack(spacing: 3) {
                            Image(systemName: "arrow.clockwise")
                            Text("Refresh")
                        }
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.appAccent)
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Color.appSecondaryBackground)

            Divider()

            // Main Content Area
            if let error = viewModel.transcriptError, viewModel.transcriptTurns.isEmpty {
                VStack(spacing: 12) {
                    Image(systemName: "bubble.left.and.exclamationmark.bubble.right")
                        .font(.system(size: 40))
                        .foregroundColor(.secondary)
                    Text("No Agent Transcript Detected")
                        .font(.headline)
                        .foregroundColor(.primary)
                    Text(error)
                        .font(.caption)
                        .multilineTextAlignment(.center)
                        .foregroundColor(.secondary)
                        .padding(.horizontal, 24)

                    Button(action: {
                        viewModel.loadTranscript()
                    }) {
                        HStack(spacing: 6) {
                            Image(systemName: "magnifyingglass")
                            Text("Detect Active Transcript")
                        }
                        .font(.system(size: 13, weight: .semibold))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .background(Color.appAccent)
                        .foregroundColor(.white)
                        .cornerRadius(8)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.appBackground)
            } else if viewModel.transcriptTurns.isEmpty && viewModel.isTranscriptLoading {
                VStack(spacing: 12) {
                    ProgressView()
                        .scaleEffect(1.2)
                    Text("Reading agent transcript from host...")
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.appBackground)
            } else if viewModel.transcriptTurns.isEmpty {
                VStack(spacing: 12) {
                    Image(systemName: "bubble.left.and.bubble.right")
                        .font(.system(size: 40))
                        .foregroundColor(.secondary)
                    Text("Ready for AI Agent Chat")
                        .font(.headline)
                        .foregroundColor(.primary)
                    Text("When Claude Code, Google Antigravity, or an Ollama agent runs in tmux, its conversation stream will appear here.")
                        .font(.caption)
                        .multilineTextAlignment(.center)
                        .foregroundColor(.secondary)
                        .padding(.horizontal, 32)

                    Button(action: {
                        viewModel.loadTranscript()
                    }) {
                        Text("Check for Active Agent")
                            .font(.system(size: 13, weight: .semibold))
                            .padding(.horizontal, 16)
                            .padding(.vertical, 8)
                            .background(Color.appAccent)
                            .foregroundColor(.white)
                            .cornerRadius(8)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.appBackground)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 12) {
                            ForEach(viewModel.transcriptTurns, id: \.id) { turn in
                                TranscriptTurnCard(
                                    turn: turn,
                                    isThinkingExpanded: expandedThinkingTurns.contains(turn.id),
                                    expandedToolCalls: $expandedToolCalls,
                                    onToggleThinking: {
                                        if expandedThinkingTurns.contains(turn.id) {
                                            expandedThinkingTurns.remove(turn.id)
                                        } else {
                                            expandedThinkingTurns.insert(turn.id)
                                        }
                                    }
                                )
                                .id(turn.id)
                            }
                            Color.clear.frame(height: 1).id(Self.bottomId)
                                .onAppear { atBottom = true; unseenNew = false }
                                .onDisappear { atBottom = false }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 12)
                    }
                    .defaultScrollAnchor(.bottom)
                    .scrollDismissesKeyboard(.interactively)
                    .background(Color.appBackground)
                    .onChange(of: viewModel.transcriptTurns) { _, _ in
                        // Follow the output only while the reader is at the bottom; otherwise
                        // leave the view alone and offer a way back.
                        if atBottom {
                            proxy.scrollTo(Self.bottomId, anchor: .bottom)
                        } else {
                            unseenNew = true
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if !atBottom {
                            Button {
                                proxy.scrollTo(Self.bottomId, anchor: .bottom)
                            } label: {
                                Label("Latest", systemImage: "arrow.down")
                                    .font(.system(size: 12, weight: .semibold))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(.regularMaterial, in: Capsule())
                                    .overlay(alignment: .topTrailing) {
                                        if unseenNew {
                                            Circle().fill(Color.appAccent).frame(width: 8, height: 8)
                                        }
                                    }
                            }
                            .padding(10)
                        }
                    }
                }
            }

            Divider()

            // Bottom prompt: a multi-line editor with a few control keys above it.
            VStack(spacing: 6) {
                HStack(spacing: 6) {
                    feedKey("Esc") { viewModel.sendInput(Data([0x1b])) }
                    feedKey("^C") { viewModel.sendInput(Data([0x03])) }
                    feedKey("↑") { viewModel.sendInput(Data([0x1b, 0x5b, 0x41])) }
                    feedKey("⏎") { viewModel.sendInput(Data([0x0d])) }
                    Spacer()
                }
                HStack(alignment: .bottom, spacing: 8) {
                    TextField("Message…", text: $promptText, axis: .vertical)
                        .lineLimit(1...6)
                        .focused($inputFocused)
                        .font(.system(size: 15))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(Color.appSecondaryBackground)
                        .cornerRadius(10)
                        .overlay(
                            RoundedRectangle(cornerRadius: 10)
                                .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                        )

                    Button(action: sendPrompt) {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.system(size: 30))
                            .foregroundColor(canSend ? .appAccent : .secondary)
                    }
                    .disabled(!canSend)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color.appBackground)
        }
        .onAppear {
            if viewModel.transcriptTurns.isEmpty {
                viewModel.loadTranscript()
            }
        }
        // Live tail: only while this view is on screen (the task ends with it).
        .task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await viewModel.reloadTranscriptQuietly()
            }
        }
    }

    private static let bottomId = "feed-bottom"

    private var canSend: Bool {
        !promptText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func feedKey(_ title: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 12, weight: .semibold, design: .monospaced))
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
    }

    private func sendPrompt() {
        guard canSend else { return }
        viewModel.sendInput(Data(FeedSubmit.bytes(for: promptText)))
        promptText = ""
        // The transcript catches up through the live tail.
        Task {
            try? await Task.sleep(for: .seconds(1))
            await viewModel.reloadTranscriptQuietly()
        }
    }
}

/// What to type into the pane for a prompt written in the feed.
enum FeedSubmit {
    /// One line: the text and Enter. Several lines: a bracketed paste (so the agent takes them as
    /// one prompt, not one prompt per line) and Enter. An end-of-paste marker inside the text is
    /// removed, so pasted content cannot leave the paste early and run as keystrokes.
    static func bytes(for text: String) -> [UInt8] {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let clean = trimmed
            .replacingOccurrences(of: "\u{1b}[201~", with: "")
            .replacingOccurrences(of: "\u{1b}[200~", with: "")
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        var out: [UInt8] = []
        if clean.contains("\n") {
            out += Array("\u{1b}[200~".utf8) + Array(clean.utf8) + Array("\u{1b}[201~".utf8)
        } else {
            out += Array(clean.utf8)
        }
        out.append(0x0d)
        return out
    }
}

// MARK: - Turn Card Component

private struct TranscriptTurnCard: View {
    let turn: TranscriptTurn
    let isThinkingExpanded: Bool
    @Binding var expandedToolCalls: Set<String>
    let onToggleThinking: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // Turn Header: Role + Timestamp + Status
            HStack(spacing: 6) {
                switch turn.turnType {
                case .user:
                    Image(systemName: "person.circle.fill")
                        .foregroundColor(.blue)
                    Text("User")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundColor(.blue)
                case .assistant:
                    Image(systemName: "sparkles")
                        .foregroundColor(.purple)
                    Text("Agent")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundColor(.purple)
                case .approvalRequest:
                    Image(systemName: "shield.lefthalf.filled")
                        .foregroundColor(.orange)
                    Text("Approval Request")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundColor(.orange)
                case .system:
                    Image(systemName: "gearshape.fill")
                        .foregroundColor(.secondary)
                    Text("System")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundColor(.secondary)
                }

                Spacer()

                if let time = turn.timestamp {
                    Text(formatTimestamp(time))
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }

                switch turn.status {
                case .running:
                    ProgressView()
                        .scaleEffect(0.6)
                case .done:
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 11))
                        .foregroundColor(.green)
                case .error:
                    Image(systemName: "exclamationmark.circle.fill")
                        .font(.system(size: 11))
                        .foregroundColor(.red)
                }
            }

            // Thinking Accordion
            if let thinking = turn.thinking, !thinking.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Button(action: onToggleThinking) {
                        HStack(spacing: 4) {
                            Image(systemName: "brain.head.profile")
                                .font(.system(size: 11))
                            Text("Thinking (\(thinking.components(separatedBy: "\n").count) steps)")
                                .font(.system(size: 11, weight: .medium))
                            Spacer()
                            Image(systemName: isThinkingExpanded ? "chevron.up" : "chevron.down")
                                .font(.system(size: 9))
                        }
                        .foregroundColor(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color.secondary.opacity(0.1))
                        .cornerRadius(6)
                    }

                    if isThinkingExpanded {
                        Text(thinking)
                            .textSelection(.enabled)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundColor(.secondary)
                            .padding(8)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.black.opacity(0.3))
                            .cornerRadius(6)
                    }
                }
            }

            // Text Content
            if let content = turn.content, !content.isEmpty {
                FeedMarkdownView(text: content)
            }

            // Tool Calls
            if !turn.toolCalls.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(turn.toolCalls, id: \.callId) { tc in
                        let isExpanded = expandedToolCalls.contains(tc.callId)
                        ToolCallCard(
                            toolCall: tc,
                            isExpanded: isExpanded,
                            onToggle: {
                                if isExpanded {
                                    expandedToolCalls.remove(tc.callId)
                                } else {
                                    expandedToolCalls.insert(tc.callId)
                                }
                            }
                        )
                    }
                }
                .padding(.top, 2)
            }
        }
        .padding(10)
        .background(turn.turnType == .user ? Color.blue.opacity(0.08) : Color.appSecondaryBackground)
        .cornerRadius(10)
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(turn.turnType == .user ? Color.blue.opacity(0.2) : Color.secondary.opacity(0.15), lineWidth: 1)
        )
    }

    private func formatTimestamp(_ raw: String) -> String {
        if raw.contains("T") {
            let parts = raw.components(separatedBy: "T")
            if parts.count > 1 {
                return parts[1].prefix(8).description
            }
        }
        return raw
    }
}

// MARK: - Tool Call Component

private struct ToolCallCard: View {
    let toolCall: TranscriptToolCall
    let isExpanded: Bool
    let onToggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button(action: onToggle) {
                HStack(spacing: 6) {
                    Image(systemName: toolIcon(name: toolCall.toolName))
                        .font(.system(size: 11))
                        .foregroundColor(toolColor(name: toolCall.toolName))

                    Text(toolCall.toolName)
                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                        .foregroundColor(.primary)

                    Text(toolCall.argumentsSummary)
                        .font(.system(size: 11, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundColor(.secondary)

                    Spacer()

                    switch toolCall.status {
                    case .running:
                        ProgressView()
                            .scaleEffect(0.5)
                    case .done:
                        Image(systemName: "checkmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundColor(.green)
                    case .error:
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundColor(.red)
                    }

                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(.system(size: 8))
                        .foregroundColor(.secondary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(Color.secondary.opacity(0.08))
                .cornerRadius(6)
            }

            if isExpanded {
                VStack(alignment: .leading, spacing: 6) {
                    if let fullArgs = toolCall.fullArgumentsJson {
                        Text("Arguments:")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundColor(.secondary)
                        Text(fullArgs)
                            .textSelection(.enabled)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.primary)
                            .padding(6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.black.opacity(0.35))
                            .cornerRadius(4)
                    }

                    if let res = toolCall.result {
                        Text("Output:")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundColor(toolCall.isError ? .red : .secondary)
                        Text(res)
                            .textSelection(.enabled)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(toolCall.isError ? .red : .primary)
                            .padding(6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.black.opacity(0.35))
                            .cornerRadius(4)
                    }
                }
                .padding(8)
                .background(Color.black.opacity(0.2))
                .cornerRadius(6)
            }
        }
    }

    private func toolIcon(name: String) -> String {
        switch name.lowercased() {
        case "run_command", "bash", "execute": return "terminal.fill"
        case "view_file", "read_file": return "doc.text.fill"
        case "write_to_file", "write_file", "replace_file_content": return "pencil.circle.fill"
        case "search_web", "read_url_content": return "globe"
        default: return "wrench.and.screwdriver.fill"
        }
    }

    private func toolColor(name: String) -> Color {
        switch name.lowercased() {
        case "run_command", "bash": return .orange
        case "write_to_file", "replace_file_content": return .purple
        case "view_file", "read_file": return .blue
        case "search_web": return .teal
        default: return .secondary
        }
    }
}
