import SwiftUI
import SshCoreBridge

public struct AiAssistantSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var userPrompt: String = ""
    @State private var chatInput: String = ""
    @State private var mode: Mode = .command

    private enum Mode: String, CaseIterable { case command = "Command", chat = "Chat" }
    @AppStorage("ai_last_model") private var lastModel: String = ""
    @State private var selectedModel: String = ""
    @State private var check: OllamaCheckState = .checking
    @State private var checkToken = 0
    @State private var copiedHint: String? = nil
    @State private var showCopiedAlert: Bool = false
    @State private var showDestructiveConfirm: Bool = false

    /// Result of asking the host what it has; drives the whole "host model" area.
    private enum OllamaCheckState: Equatable {
        case checking
        case notInstalled
        case notRunning(version: String?)
        case noModels
        case ready(models: [String])
        case failed(String)
    }

    private var isReady: Bool {
        if case .ready = check { return !selectedModel.isEmpty }
        return false
    }

    private let quickPrompts = [
        "Show open ports & listeners",
        "Find files modified today",
        "Disk usage by top directories",
        "Kill process on port 3000",
        "Revert last git commit keeping changes"
    ]

    public init(viewModel: TerminalSessionViewModel, onClose: (() -> Void)? = nil) {
        self.viewModel = viewModel
        self.onClose = onClose
    }

    private func closeSheet() {
        // Leaving while a request runs would leave the model generating for nobody.
        if viewModel.isAiLoading { viewModel.stopAiQuery() }
        if viewModel.isChatLoading { viewModel.stopChat() }
        onClose?()
        dismiss()
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
            Picker("Mode", selection: $mode) {
                ForEach(Mode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.top, 8)
            if mode == .chat {
                chatBody
            } else {
            ScrollView {
                VStack(spacing: 18) {
                    headerSection
                    hostCheckSection
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
            }
            }
            .background(Color(red: 0.08, green: 0.08, blue: 0.10).ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") {
                        closeSheet()
                    }
                    .foregroundColor(.secondary)
                }
            }
            .task { await runCheck() }
            .alert("Destructive Command", isPresented: $showDestructiveConfirm) {
                Button("Cancel", role: .cancel) {}
                Button("Run Anyway", role: .destructive) {
                    if let cmd = viewModel.activeAiSuggestion?.command {
                        viewModel.executeAiCommand(cmd)
                        closeSheet()
                    }
                }
            } message: {
                Text("This command contains potentially destructive actions (e.g. recursive file deletion or hard reset). Are you sure you want to run it immediately?")
            }
        }
        .interactiveDismissDisabled()
    }

    // MARK: - Chat
    private var chatBody: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                hostCheckSection
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        if viewModel.chatMessages.isEmpty {
                            Text("Ask the local model anything about your work: commands, errors, code. It answers from the host's Ollama and cannot run anything.")
                                .font(.caption)
                                .foregroundColor(.secondary)
                                .padding(.top, 20)
                        }
                        ForEach(Array(viewModel.chatMessages.enumerated()), id: \.offset) { index, message in
                            chatBubble(message).id(index)
                        }
                        if viewModel.isChatLoading {
                            HStack(spacing: 8) {
                                ProgressView().scaleEffect(0.8)
                                Text("Thinking…").font(.caption).foregroundColor(.secondary)
                            }
                            .id("loading")
                        }
                        if let error = viewModel.chatError {
                            errorSection(error)
                        }
                    }
                    .padding(16)
                }
                .onChange(of: viewModel.chatMessages.count) { _, _ in
                    withAnimation { proxy.scrollTo(viewModel.chatMessages.count - 1, anchor: .bottom) }
                }
                .onChange(of: viewModel.isChatLoading) { _, loading in
                    if loading { withAnimation { proxy.scrollTo("loading", anchor: .bottom) } }
                }
            }

            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message…", text: $chatInput, axis: .vertical)
                    .lineLimit(1...5)
                    .font(.system(size: 14))
                    .foregroundColor(.white)
                    .padding(10)
                    .background(Color(white: 0.12))
                    .cornerRadius(10)
                Button {
                    if viewModel.isChatLoading {
                        viewModel.stopChat()
                    } else {
                        viewModel.sendChat(chatInput, modelName: selectedModel)
                        chatInput = ""
                    }
                } label: {
                    Image(systemName: viewModel.isChatLoading ? "stop.fill" : "arrow.up")
                        .font(.system(size: 16, weight: .bold))
                        .frame(width: 40, height: 40)
                        .background(viewModel.isChatLoading ? Color.red : Color.purple)
                        .foregroundColor(.white)
                        .clipShape(Circle())
                }
                .disabled(!viewModel.isChatLoading
                          && (chatInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !isReady))
                .opacity(!viewModel.isChatLoading
                         && (chatInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !isReady) ? 0.4 : 1)
                Menu {
                    Button("New chat", role: .destructive) { viewModel.clearChat() }
                        .disabled(viewModel.chatMessages.isEmpty)
                } label: {
                    Image(systemName: "ellipsis.circle").font(.system(size: 22)).foregroundColor(.secondary)
                        .frame(width: 32, height: 40)
                }
            }
            .padding(12)
        }
    }

    private func chatBubble(_ message: ChatMessage) -> some View {
        let isUser = message.role == "user"
        return HStack {
            if isUser { Spacer(minLength: 40) }
            Text(message.content)
                .font(.system(size: 14))
                .foregroundColor(.white)
                .textSelection(.enabled)
                .padding(10)
                .background(isUser ? Color.purple.opacity(0.35) : Color(white: 0.14))
                .cornerRadius(12)
            if !isUser { Spacer(minLength: 40) }
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

    // MARK: - Host check
    private var hostCheckSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("HOST MODEL")
                    .font(.system(size: 11, weight: .bold, design: .monospaced))
                    .foregroundColor(.secondary)
                Spacer()
                Button {
                    Task { await runCheck() }
                } label: {
                    Label("Re-check", systemImage: "arrow.clockwise").font(.caption)
                }
                .disabled(check == .checking)
            }

            switch check {
            case .checking:
                HStack(spacing: 8) {
                    ProgressView().scaleEffect(0.8)
                    Text("Checking Ollama on the host…").font(.caption).foregroundColor(.secondary)
                }
            case .notInstalled:
                problemCard(
                    "Ollama is not installed on this host.",
                    detail: "Install it on the host, then Re-check.",
                    commands: ["brew install ollama"],
                    footnote: "or download it from ollama.com/download")
            case .notRunning(let version):
                problemCard(
                    "Ollama is installed\(version.map { " (v\($0))" } ?? "") but not reachable on 127.0.0.1:11434.",
                    detail: "Start the server on the host (or open the Ollama app), then Re-check.",
                    commands: ["ollama serve"],
                    footnote: nil)
            case .noModels:
                problemCard(
                    "Ollama is running but has no models yet.",
                    detail: "Pull one on the host, then Re-check.",
                    commands: ["ollama pull qwen2.5-coder:7b"],
                    footnote: nil)
            case .failed(let message):
                problemCard("Could not check the host.", detail: message, commands: [], footnote: nil)
            case .ready(let models):
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(models, id: \.self) { model in
                            Button {
                                selectedModel = model
                                lastModel = model
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
    }

    private func problemCard(_ title: String, detail: String, commands: [String], footnote: String?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(title, systemImage: "exclamationmark.triangle.fill")
                .font(.caption.weight(.semibold))
                .foregroundColor(.orange)
            Text(detail).font(.caption).foregroundColor(.secondary)
            ForEach(commands, id: \.self) { cmd in
                HStack {
                    Text(cmd)
                        .font(.system(size: 12, design: .monospaced))
                        .textSelection(.enabled)
                    Spacer()
                    Button(copiedHint == cmd ? "Copied" : "Copy") {
                        #if os(iOS)
                        UIPasteboard.general.string = cmd
                        #endif
                        copiedHint = cmd
                    }
                    .font(.caption.bold())
                }
                .padding(8)
                .background(Color(white: 0.12))
                .cornerRadius(8)
            }
            if let footnote {
                Text(footnote).font(.caption2).foregroundColor(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.10))
        .cornerRadius(10)
    }

    /// Asks the host (installed? server up? which models?). A stale answer never overwrites a
    /// newer one: each run carries a token.
    private func runCheck() async {
        checkToken += 1
        let token = checkToken
        check = .checking
        copiedHint = nil
        guard let handle = viewModel.sessionHandle, viewModel.state == .connected else {
            check = .failed("Not connected to a host.")
            return
        }
        let result: OllamaCheckState
        do {
            let status = try await handle.checkHostOllama()
            if !status.installed {
                result = .notInstalled
            } else if !status.running {
                result = .notRunning(version: status.version)
            } else if status.models.isEmpty {
                result = .noModels
            } else {
                result = .ready(models: status.models)
            }
            if token == checkToken, case .ready(let models) = result {
                selectedModel = models.contains(lastModel) ? lastModel : (models.first ?? "")
            }
        } catch {
            result = .failed(error.localizedDescription)
        }
        if token == checkToken { check = result }
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
                if viewModel.isAiLoading {
                    viewModel.stopAiQuery()
                } else {
                    viewModel.startAiQuery(userPrompt: userPrompt, modelName: selectedModel, targetOs: "macOS")
                }
            } label: {
                HStack {
                    if viewModel.isAiLoading {
                        Image(systemName: "stop.fill")
                    } else {
                        Image(systemName: "sparkles")
                    }
                    Text(viewModel.isAiLoading ? "Stop" : "Generate Command")
                        .fontWeight(.semibold)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(viewModel.isAiLoading ? Color.red : (userPrompt.trimmingCharacters(in: .whitespaces).isEmpty ? Color.purple.opacity(0.4) : Color.purple))
                .foregroundColor(.white)
                .cornerRadius(10)
            }
            .disabled(!viewModel.isAiLoading && (userPrompt.trimmingCharacters(in: .whitespaces).isEmpty || !isReady))
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
                    closeSheet()
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
                        closeSheet()
                    }
                } label: {
                    HStack {
                        Image(systemName: "terminal.fill")
                        Text("Run ⏎")
                    }
                    .font(.subheadline.bold())
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(suggestion.riskLevel == .destructive ? Color.red : Color.appAccent)
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
