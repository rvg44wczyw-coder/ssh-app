import Foundation
import SshCoreBridge

/// The assistant's chat and command state for one server. It belongs to the server, not to a
/// terminal tab: closing a tab or the assistant screen keeps the history. Nothing is written to
/// disk (a chat can hold pasted secrets); it is lost when the app is killed.
///
/// A request borrows the connection of a terminal session on that server (`handle`) for its
/// duration only. When the app goes to the background the sockets close; a running request then
/// ends with a visible error and the history stays.
@MainActor
public final class AssistantSession: ObservableObject, Identifiable {
    public nonisolated let serverId: UUID
    public nonisolated var id: UUID { serverId }

    @Published public var activeAiSuggestion: AiCommandSuggestion? = nil
    @Published public var isAiLoading = false
    @Published public var aiError: String? = nil
    @Published public var chatMessages: [ChatMessage] = []
    @Published public var isChatLoading = false
    @Published public var chatError: String? = nil
    private var aiTask: Task<Void, Never>? = nil
    private var chatTask: Task<Void, Never>? = nil

    public init(serverId: UUID) { self.serverId = serverId }

    public var hasHistory: Bool { !chatMessages.isEmpty || activeAiSuggestion != nil }

    /// One line for the card: the last answer, else the last suggested command.
    public var preview: String {
        if let last = chatMessages.last(where: { $0.role == "assistant" }) { return last.content }
        return activeAiSuggestion?.command ?? ""
    }

    private static let chatSystemPrompt =
        "You are a concise assistant for a developer working on their own computer through a remote terminal. "
        + "Answer questions about commands, code, tools and errors. Reply in the language of the question. "
        + "You cannot run anything yourself; put commands in fenced code blocks so they can be copied."

    /// Keeps the request small: the newest turns that fit, always starting with a user turn.
    private func chatWindow() -> [ChatMessage] {
        var kept: [ChatMessage] = []
        var bytes = 0
        for m in chatMessages.reversed() {
            bytes += m.content.utf8.count
            if bytes > 60_000 || kept.count >= 40 { break }
            kept.append(m)
        }
        kept.reverse()
        while let first = kept.first, first.role != "user" { kept.removeFirst() }
        return kept
    }

    public func sendChat(_ text: String, modelName: String, handle: SshSessionHandle?, connected: Bool) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !isChatLoading else { return }
        chatMessages.append(ChatMessage(role: "user", content: trimmed))
        chatError = nil
        guard let handle, connected else {
            chatError = "Session is not connected"
            return
        }
        isChatLoading = true
        let window = chatWindow()
        chatTask = Task { [weak self] in
            do {
                let reply = try await handle.chatHostOllama(
                    model: modelName, systemPrompt: Self.chatSystemPrompt, messages: window)
                guard let self, !Task.isCancelled else { return }
                self.chatMessages.append(ChatMessage(role: "assistant", content: reply))
                self.isChatLoading = false
            } catch {
                guard let self, !Task.isCancelled else { return }
                self.chatError = error.localizedDescription
                self.isChatLoading = false
            }
        }
    }

    /// Stop drops the call (the host kills the request); the unanswered question stays in the
    /// history so it can be edited and sent again.
    public func stopChat() {
        chatTask?.cancel()
        chatTask = nil
        isChatLoading = false
    }

    public func clearChat() {
        stopChat()
        chatMessages = []
        chatError = nil
    }

    /// Starts a request that Stop can cancel (the core then closes the channel and the host kills
    /// the request, so the model stops generating).
    public func startAiQuery(
        userPrompt: String, modelName: String, targetOs: String = "macOS",
        handle: SshSessionHandle?, connected: Bool, recentLines: [String]
    ) {
        aiTask?.cancel()
        guard let handle, connected else {
            aiError = "Session is not connected"
            return
        }
        isAiLoading = true
        aiError = nil
        let systemPrompt = assembleAiSystemPrompt(targetOs: targetOs, shellName: "zsh", cwd: nil)
        let request = AiCommandRequest(
            userPrompt: userPrompt, targetOs: targetOs, shellName: "zsh", cwd: nil, terminalContext: recentLines)
        let assembled = assembleAiUserPrompt(request: request)
        aiTask = Task { [weak self] in
            do {
                let suggestion = try await handle.queryHostOllama(
                    model: modelName, systemPrompt: systemPrompt, userPrompt: assembled)
                guard let self, !Task.isCancelled else { return }
                self.activeAiSuggestion = suggestion
                self.isAiLoading = false
            } catch {
                // Stop already reset the state; a late failure of the cancelled call is not an error.
                guard let self, !Task.isCancelled else { return }
                self.aiError = error.localizedDescription
                self.isAiLoading = false
            }
        }
    }

    public func stopAiQuery() {
        aiTask?.cancel()
        aiTask = nil
        isAiLoading = false
        aiError = nil
    }

    public func clearAll() {
        clearChat()
        stopAiQuery()
        activeAiSuggestion = nil
    }

    /// The app is leaving the foreground: its sockets are about to close, so a running request
    /// cannot finish. Ends it with a visible error; the history stays and nothing is retried.
    public func interrupt() {
        if isChatLoading {
            stopChat()
            chatError = "Interrupted: the app went to the background. Send the question again."
        }
        if isAiLoading {
            aiTask?.cancel()
            aiTask = nil
            isAiLoading = false
            aiError = "Interrupted: the app went to the background. Try again."
        }
    }
}

/// One assistant per server, created on first use. A server's card appears on the sessions screen
/// once its assistant has something to show.
@MainActor
public final class AssistantStore: ObservableObject {
    @Published public private(set) var sessions: [UUID: AssistantSession] = [:]

    public func session(for serverId: UUID) -> AssistantSession {
        if let s = sessions[serverId] { return s }
        let s = AssistantSession(serverId: serverId)
        sessions[serverId] = s
        return s
    }

    /// Servers whose card is shown: the ones with history, in the order of `serverIds`.
    public func withHistory(in serverIds: [UUID]) -> [AssistantSession] {
        serverIds.compactMap { sessions[$0] }.filter(\.hasHistory)
    }

    public func interruptAll() { sessions.values.forEach { $0.interrupt() } }

    /// Drops the assistant of a server that was deleted.
    public func remove(serverId: UUID) {
        sessions[serverId]?.clearAll()
        sessions[serverId] = nil
    }
}
