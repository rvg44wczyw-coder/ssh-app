import SwiftUI
import SshCoreBridge

public struct StructuredTranscriptView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var promptText: String = ""
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
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 12)
                    }
                    .background(Color.appBackground)
                    .onChange(of: viewModel.transcriptTurns.count) { _, _ in
                        if let lastId = viewModel.transcriptTurns.last?.id {
                            withAnimation {
                                proxy.scrollTo(lastId, anchor: .bottom)
                            }
                        }
                    }
                }
            }

            Divider()

            // Bottom Prompt Input Bar
            HStack(spacing: 8) {
                TextField("Send instruction to agent...", text: $promptText)
                    .textFieldStyle(PlainTextFieldStyle())
                    .font(.system(size: 13))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(Color.appSecondaryBackground)
                    .cornerRadius(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                    )

                Button(action: {
                    sendPrompt()
                }) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 26))
                        .foregroundColor(promptText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .secondary : .appAccent)
                }
                .disabled(promptText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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
    }

    private func sendPrompt() {
        let trimmed = promptText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let payload = "\(trimmed)\n"
        if let data = payload.data(using: .utf8) {
            viewModel.sendInput(data)
        }
        promptText = ""
        // Schedule refresh after brief delay to observe agent pickup
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            viewModel.loadTranscript()
        }
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
                Text(content)
                    .font(.system(size: 13))
                    .foregroundColor(.primary)
                    .fixedSize(horizontal: false, vertical: true)
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
