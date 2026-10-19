import SwiftUI
import SshCoreBridge

public struct StructuredTranscriptView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var promptText: String = ""
    @State private var atBottom = true
    @State private var unseenNew = false
    @State private var scrollToken = 0
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
                FeedTextView(
                    turns: viewModel.transcriptTurns,
                    expandedThinking: expandedThinkingTurns,
                    expandedTools: expandedToolCalls,
                    onToggle: { toggle in
                        switch toggle {
                        case .thinking(let id):
                            if expandedThinkingTurns.contains(id) { expandedThinkingTurns.remove(id) } else { expandedThinkingTurns.insert(id) }
                        case .tool(let id):
                            if expandedToolCalls.contains(id) { expandedToolCalls.remove(id) } else { expandedToolCalls.insert(id) }
                        }
                    },
                    atBottom: $atBottom,
                    scrollToken: scrollToken
                )
                .background(Color.appBackground)
                .onChange(of: viewModel.transcriptTurns) { _, _ in
                    // The text view follows the output only while the reader is at the bottom.
                    if !atBottom { unseenNew = true }
                }
                .onChange(of: atBottom) { _, now in if now { unseenNew = false } }
                .overlay(alignment: .bottomTrailing) {
                    if !atBottom {
                        Button { scrollToken += 1 } label: {
                            Label("Latest", systemImage: "arrow.down")
                                .font(.system(size: 12, weight: .semibold))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(.regularMaterial, in: Capsule())
                                .overlay(alignment: .topTrailing) {
                                    if unseenNew { Circle().fill(Color.appAccent).frame(width: 8, height: 8) }
                                }
                        }
                        .padding(10)
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
