import SwiftUI
import SshCoreBridge

public struct AiAssistantSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Environment(\.dismiss) private var dismiss

    @State private var userPrompt: String = ""
    @State private var selectedModel: String = "qwen2.5-coder:7b"
    @State private var showCopiedAlert: Bool = false
    @State private var showDestructiveConfirm: Bool = false

    private let availableModels = [
        "qwen2.5-coder:7b",
        "llama3.2",
        "deepseek-coder",
        "mistral"
    ]

    private let quickPrompts = [
        "Show open ports & listeners",
        "Find files modified today",
        "Disk usage by top directories",
        "Kill process on port 3000",
        "Revert last git commit keeping changes"
    ]

    public init(viewModel: TerminalSessionViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 18) {
                    headerSection
                    modelSelectorRow
                    promptInputSection
                    quickPromptChips

                    if viewModel.isAiLoading {
                        loadingSection
                    }

                    if let error = viewModel.aiError {
                        errorSection(error)
                    }

                    if let suggestion = viewModel.activeAiSuggestion {
                        suggestionCard(suggestion)
                    }
                }
                .padding(16)
            }
            .background(Color(red: 0.08, green: 0.08, blue: 0.10).ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") {
                        viewModel.closeAiAssistant()
                        dismiss()
                    }
                    .foregroundColor(.secondary)
                }
            }
            .alert("Destructive Command", isPresented: $showDestructiveConfirm) {
                Button("Cancel", role: .cancel) {}
                Button("Run Anyway", role: .destructive) {
                    if let cmd = viewModel.activeAiSuggestion?.command {
                        viewModel.executeAiCommand(cmd)
                        dismiss()
                    }
                }
            } message: {
                Text("This command contains potentially destructive actions (e.g. recursive file deletion or hard reset). Are you sure you want to run it immediately?")
            }
        }
    }

    // MARK: - Header
    private var headerSection: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(Color.purple.opacity(0.2))
                    .frame(width: 44, height: 44)
                Image(systemName: "wand.and.stars")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundColor(.purple)
            }

            VStack(alignment: .leading, spacing: 2) {
                Text("AI Shell Assistant")
                    .font(.headline)
                    .foregroundColor(.white)
                Text("Generates shell commands via Host Ollama over SSH")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            Spacer()
        }
    }

    // MARK: - Model Selector
    private var modelSelectorRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("HOST MODEL")
                .font(.system(size: 11, weight: .bold, design: .monospaced))
                .foregroundColor(.secondary)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(availableModels, id: \.self) { model in
                        Button {
                            selectedModel = model
                        } label: {
                            Text(model)
                                .font(.system(size: 12, weight: .medium, design: .monospaced))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(selectedModel == model ? Color.purple.opacity(0.3) : Color(white: 0.15))
                                .foregroundColor(selectedModel == model ? .purple : .secondary)
                                .cornerRadius(8)
                                .overlay(
                                    RoundedRectangle(cornerRadius: 8)
                                        .stroke(selectedModel == model ? Color.purple : Color.clear, lineWidth: 1)
                                )
                        }
                    }
                }
            }
        }
    }

    // MARK: - Prompt Input
    private var promptInputSection: some View {
        VStack(spacing: 10) {
            HStack(alignment: .top) {
                TextField("Describe what you want to achieve...", text: $userPrompt, axis: .vertical)
                    .lineLimit(2...5)
                    .font(.system(size: 14))
                    .foregroundColor(.white)
                    .padding(12)
                    .background(Color(white: 0.12))
                    .cornerRadius(10)
            }

            Button {
                Task {
                    await viewModel.queryAiAssistant(
                        userPrompt: userPrompt,
                        modelName: selectedModel,
                        targetOs: "macOS"
                    )
                }
            } label: {
                HStack {
                    if viewModel.isAiLoading {
                        ProgressView()
                            .progressViewStyle(CircularProgressViewStyle(tint: .white))
                            .scaleEffect(0.8)
                    } else {
                        Image(systemName: "sparkles")
                    }
                    Text(viewModel.isAiLoading ? "Consulting Host Ollama..." : "Generate Command")
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(userPrompt.trimmingCharacters(in: .whitespaces).isEmpty ? Color.purple.opacity(0.4) : Color.purple)
                .foregroundColor(.white)
                .cornerRadius(10)
            }
            .disabled(userPrompt.trimmingCharacters(in: .whitespaces).isEmpty || viewModel.isAiLoading)
        }
    }

    // MARK: - Quick Prompts
    private var quickPromptChips: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("SUGGESTIONS")
                .font(.system(size: 11, weight: .bold, design: .monospaced))
                .foregroundColor(.secondary)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(quickPrompts, id: \.self) { prompt in
                        Button {
                            userPrompt = prompt
                        } label: {
                            Text(prompt)
                                .font(.system(size: 11))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(Color(white: 0.14))
                                .foregroundColor(.white.opacity(0.85))
                                .cornerRadius(12)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Loading
    private var loadingSection: some View {
        HStack(spacing: 12) {
            ProgressView()
                .progressViewStyle(CircularProgressViewStyle(tint: .purple))
            Text("Querying Ollama at 127.0.0.1:11434...")
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .padding(.vertical, 8)
    }

    // MARK: - Error
    private func errorSection(_ err: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundColor(.red)
            Text(err)
                .font(.caption)
                .foregroundColor(.red)
            Spacer()
        }
        .padding(12)
        .background(Color.red.opacity(0.12))
        .cornerRadius(8)
    }

    // MARK: - Suggestion Card
    private func suggestionCard(_ suggestion: AiCommandSuggestion) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            // Risk Badge Header
            HStack {
                riskBadge(suggestion.riskLevel)
                Spacer()
                Button {
                    UIPasteboard.general.string = suggestion.command
                    showCopiedAlert = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        showCopiedAlert = false
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: showCopiedAlert ? "checkmark" : "doc.on.doc")
                        Text(showCopiedAlert ? "Copied" : "Copy")
                    }
                    .font(.caption.bold())
                    .foregroundColor(.secondary)
                }
            }

            // Warnings if any
            if !suggestion.warnings.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(suggestion.warnings, id: \.self) { warn in
                        HStack(alignment: .top, spacing: 6) {
                            Image(systemName: "exclamationmark.octagon.fill")
                                .font(.caption)
                                .foregroundColor(.yellow)
                            Text(warn)
                                .font(.caption)
                                .foregroundColor(.yellow)
                        }
                    }
                }
                .padding(8)
                .background(Color.yellow.opacity(0.1))
                .cornerRadius(6)
            }

            // Command Code Box
            VStack(alignment: .leading, spacing: 4) {
                Text(suggestion.command)
                    .font(.system(size: 13, weight: .regular, design: .monospaced))
                    .foregroundColor(Color.green.opacity(0.95))
                    .textSelection(.enabled)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.black.opacity(0.6))
                    .cornerRadius(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(Color(white: 0.2), lineWidth: 1)
                    )
            }

            // Explanation
            Text(suggestion.explanation)
                .font(.caption)
                .foregroundColor(.secondary)
                .lineSpacing(3)

            // Action Buttons
            HStack(spacing: 12) {
                Button {
                    viewModel.insertAiCommand(suggestion.command)
                    dismiss()
                } label: {
                    HStack {
                        Image(systemName: "text.insert")
                        Text("Insert")
                    }
                    .font(.subheadline.bold())
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(Color(white: 0.2))
                    .foregroundColor(.white)
                    .cornerRadius(8)
                }

                Button {
                    if suggestion.riskLevel == .destructive {
                        showDestructiveConfirm = true
                    } else {
                        viewModel.executeAiCommand(suggestion.command)
                        dismiss()
                    }
                } label: {
                    HStack {
                        Image(systemName: "terminal.fill")
                        Text("Run ⏎")
                    }
                    .font(.subheadline.bold())
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(suggestion.riskLevel == .destructive ? Color.red : Color.blue)
                    .foregroundColor(.white)
                    .cornerRadius(8)
                }
            }
        }
        .padding(16)
        .background(Color(white: 0.12))
        .cornerRadius(12)
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color(white: 0.2), lineWidth: 1)
        )
    }

    // MARK: - Risk Badge
    @ViewBuilder
    private func riskBadge(_ level: AiRiskLevel) -> some View {
        switch level {
        case .safe:
            HStack(spacing: 4) {
                Circle().fill(Color.green).frame(width: 7, height: 7)
                Text("Safe")
                    .font(.caption2.bold())
                    .foregroundColor(.green)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.green.opacity(0.15))
            .cornerRadius(6)

        case .caution:
            HStack(spacing: 4) {
                Circle().fill(Color.yellow).frame(width: 7, height: 7)
                Text("Caution: Modifies State")
                    .font(.caption2.bold())
                    .foregroundColor(.yellow)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.yellow.opacity(0.15))
            .cornerRadius(6)

        case .elevated:
            HStack(spacing: 4) {
                Circle().fill(Color.orange).frame(width: 7, height: 7)
                Text("Elevated: Requires Sudo")
                    .font(.caption2.bold())
                    .foregroundColor(.orange)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.orange.opacity(0.15))
            .cornerRadius(6)

        case .destructive:
            HStack(spacing: 4) {
                Circle().fill(Color.red).frame(width: 7, height: 7)
                Text("Destructive: High Risk")
                    .font(.caption2.bold())
                    .foregroundColor(.red)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.red.opacity(0.15))
            .cornerRadius(6)
        }
    }
}
