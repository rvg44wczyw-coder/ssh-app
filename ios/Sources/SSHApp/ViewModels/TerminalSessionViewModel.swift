import Foundation
import Combine
import SshCoreBridge

@MainActor
public final class TerminalSessionViewModel: ObservableObject, SshSessionCallback {
    public let instanceId = String(UUID().uuidString.prefix(6))
    public var session: TabSession
    public let outputPublisher = PassthroughSubject<Data, Never>()

    @Published public private(set) var state: SessionState = .disconnected
    @Published public private(set) var errorMessage: String? = nil
    @Published public private(set) var activePortForward: PortForwardHandle? = nil

    // AI Assistant State
    @Published public var activeAiSuggestion: AiCommandSuggestion? = nil
    @Published public var isAiLoading: Bool = false
    @Published public var aiError: String? = nil
    private var aiTask: Task<Void, Never>? = nil

    // Structured Transcript / Dual-Mode State
    @Published public var isChatMode: Bool = false
    @Published public var transcriptTurns: [TranscriptTurn] = []
    @Published public var isTranscriptLoading: Bool = false
    @Published public var activeTranscriptPath: String? = nil
    @Published public var transcriptError: String? = nil

    // Smart Snippets & Fuzzy History Search State
    @Published public var remoteHistoryCommands: [String] = []
    @Published public var isHistoryLoading: Bool = false

    // Host Telemetry & Process Monitor State
    @Published public var telemetrySnapshot: HostTelemetrySnapshot? = nil
    @Published public var isTelemetryLoading: Bool = false
    @Published public var telemetryError: String? = nil

    // Remote File Browser State
    @Published public var fileListing: RemoteDirectoryListing? = nil
    @Published public var filePreview: RemoteFilePreview? = nil
    @Published public var isFileBrowserLoading: Bool = false
    @Published public var fileBrowserError: String? = nil

    private var handle: SshSessionHandle?
    /// The live core session, for host-side management actions (e.g. agent hook install).
    public var sessionHandle: SshSessionHandle? { handle }
    /// Host key fingerprint pinned for this tab's server (nil = trust on first use).
    public var pinnedHostKey: String?
    /// Called once with the fingerprint seen on a first connection, so the caller can pin it.
    public var onHostKeyObserved: ((String) -> Void)?
    private var lastCols: UInt16 = 80
    private var lastRows: UInt16 = 24

    public init(session: TabSession) {
        self.session = session
        print("[SwiftVM \(instanceId)] initialized for session '\(session.title)' (id=\(session.id))")
    }

    public func start(host: String, port: UInt16, username: String, privateKeyPEM: String) {
        print("[SwiftVM \(instanceId)] start() host=\(host), port=\(port), user=\(username), state=\(state)")
        guard state != .connected && state != .connecting else {
            print("[SwiftVM \(instanceId)] start() skipped: already \(state)")
            return
        }

        self.terminalDataBuffer = Data()

        let config = SessionConfig(
            host: host,
            port: port,
            username: username,
            privateKeyPem: privateKeyPEM,
            sessionName: session.sessionName,
            initialCols: lastCols,
            initialRows: lastRows,
            command: session.command,
            pinnedHostKey: pinnedHostKey
        )

        let sessionHandle = SshSessionHandle(config: config)
        self.handle = sessionHandle

        do {
            self.state = .connecting
            self.errorMessage = nil
            try sessionHandle.connect(callback: self)
            print("[SwiftVM] sessionHandle.connect() dispatched successfully")
        } catch {
            print("[SwiftVM] sessionHandle.connect() failed synchronously: \(error)")
            self.state = .failed(reason: error.localizedDescription)
            self.errorMessage = error.localizedDescription
        }
    }

    public func forceRestart(host: String, port: UInt16, username: String, privateKeyPEM: String) {
        print("[SwiftVM] forceRestart() resetting state...")
        handle?.disconnect()
        handle = nil
        self.state = .disconnected
        self.errorMessage = nil
        start(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM)
    }

    private var terminalDataBuffer = Data()

    public func getHistoryBuffer() -> Data {
        return terminalDataBuffer
    }

    public func sendInput(_ data: Data) {
        print("[SwiftVM \(instanceId)] sendInput: len=\(data.count), handle=\(handle != nil), state=\(state)")
        guard let handle = handle, state == .connected else {
            print("[SwiftVM \(instanceId)] sendInput dropped: handle is nil or state is \(state)")
            return
        }
        do {
            try handle.sendInput(data: data)
            print("[SwiftVM \(instanceId)] handle.sendInput succeeded")
        } catch {
            print("[SwiftVM \(instanceId)] handle.sendInput error: \(error)")
        }
    }

    public func resize(cols: UInt16, rows: UInt16) {
        self.lastCols = cols
        self.lastRows = rows
        print("[SwiftVM \(instanceId)] resize: \(cols)x\(rows), handle=\(handle != nil)")
        guard let handle = handle else { return }
        try? handle.resize(cols: cols, rows: rows)
    }

    /// Starts port forwarding to remote port.
    public func startPortForward(remotePort: UInt16, localPort: UInt16? = nil) async throws -> PortForwardHandle {
        guard let handle = handle, state == .connected else {
            throw SshCoreError.NotConnected
        }
        let forwardHandle = try await handle.startPortForward(remotePort: remotePort, localPort: localPort)
        self.activePortForward = forwardHandle
        return forwardHandle
    }

    /// Stops the currently active port forward.
    public func stopPortForward() {
        activePortForward?.stop()
        activePortForward = nil
    }

    // MARK: - AI Shell Assistant

    public func getRecentTerminalLines(maxLines: Int = 20) -> [String] {
        guard let text = String(data: terminalDataBuffer, encoding: .utf8) else { return [] }
        let lines = text.components(separatedBy: .newlines).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        return Array(lines.suffix(maxLines))
    }

    /// Starts a request that Stop can cancel (cancelling drops the call; the core then closes the
    /// channel and the host kills the request, so the model stops generating).
    public func startAiQuery(userPrompt: String, modelName: String, targetOs: String = "macOS") {
        aiTask?.cancel()
        aiTask = Task { [weak self] in
            await self?.queryAiAssistant(userPrompt: userPrompt, modelName: modelName, targetOs: targetOs)
        }
    }

    public func stopAiQuery() {
        aiTask?.cancel()
        aiTask = nil
        isAiLoading = false
        aiError = nil
    }

    public func queryAiAssistant(
        userPrompt: String,
        modelName: String = "qwen2.5-coder:7b",
        targetOs: String = "macOS"
    ) async {
        guard let handle = handle, state == .connected else {
            self.aiError = "Session is not connected"
            return
        }

        self.isAiLoading = true
        self.aiError = nil

        let recentLines = getRecentTerminalLines(maxLines: 20)
        let systemPrompt = assembleAiSystemPrompt(targetOs: targetOs, shellName: "zsh", cwd: nil)
        let request = AiCommandRequest(
            userPrompt: userPrompt,
            targetOs: targetOs,
            shellName: "zsh",
            cwd: nil,
            terminalContext: recentLines
        )
        let assembledUserPrompt = assembleAiUserPrompt(request: request)

        do {
            let suggestion = try await handle.queryHostOllama(
                model: modelName,
                systemPrompt: systemPrompt,
                userPrompt: assembledUserPrompt
            )
            guard !Task.isCancelled else { return }
            self.activeAiSuggestion = suggestion
            self.isAiLoading = false
        } catch {
            // Stop already reset the state; a late failure of the cancelled call is not an error.
            guard !Task.isCancelled else { return }
            self.aiError = error.localizedDescription
            self.isAiLoading = false
        }
    }

    public func insertAiCommand(_ command: String) {
        if let data = command.data(using: .utf8) {
            sendInput(data)
        }
    }

    public func executeAiCommand(_ command: String) {
        if let data = (command + "\r").data(using: .utf8) {
            sendInput(data)
        }
    }

    public func toggleChatMode() {
        isChatMode.toggle()
        if isChatMode && transcriptTurns.isEmpty {
            loadTranscript()
        }
    }

    public func loadTranscript() {
        guard let handle = handle, state == .connected else { return }
        isTranscriptLoading = true
        transcriptError = nil

        Task {
            do {
                var path = self.activeTranscriptPath
                if path == nil {
                    path = try await handle.detectActiveTranscriptPath(baseDir: nil)
                    self.activeTranscriptPath = path
                }

                guard let validPath = path else {
                    self.transcriptError = "No active agent transcript detected"
                    self.isTranscriptLoading = false
                    return
                }

                let turns = try await handle.readRemoteTranscript(remotePath: validPath)
                self.transcriptTurns = turns
                self.isTranscriptLoading = false
            } catch {
                self.transcriptError = error.localizedDescription
                self.isTranscriptLoading = false
            }
        }
    }

    public func fetchRemoteHistory() {
        guard let handle = handle, state == .connected else { return }
        isHistoryLoading = true
        Task {
            do {
                let cmds = try await handle.fetchRemoteShellHistory(limit: 80)
                self.remoteHistoryCommands = cmds
                self.isHistoryLoading = false
            } catch {
                self.isHistoryLoading = false
            }
        }
    }

    public func refreshHostTelemetry() {
        guard let handle = handle, state == .connected else {
            self.telemetryError = "Session is not connected"
            return
        }
        isTelemetryLoading = true
        telemetryError = nil
        Task {
            do {
                let snap = try await handle.fetchHostTelemetry()
                self.telemetrySnapshot = snap
                self.isTelemetryLoading = false
            } catch {
                self.telemetryError = error.localizedDescription
                self.isTelemetryLoading = false
            }
        }
    }

    public func killRemoteProcess(pid: UInt32, force: Bool) {
        guard let handle = handle, state == .connected else { return }
        Task {
            do {
                try await handle.killRemoteProcess(pid: pid, force: force)
                self.refreshHostTelemetry()
            } catch {
                self.telemetryError = error.localizedDescription
            }
        }
    }

    /// Lists a remote directory. `nil` path resolves the active tmux pane cwd (or `$HOME`).
    public func loadRemoteDirectory(path: String?, includeHidden: Bool) {
        guard let handle = handle, state == .connected else {
            fileBrowserError = "Session is not connected"
            return
        }
        isFileBrowserLoading = true
        fileBrowserError = nil
        Task {
            do {
                self.fileListing = try await handle.listRemoteDirectory(path: path, includeHidden: includeHidden)
            } catch {
                self.fileBrowserError = error.localizedDescription
            }
            self.isFileBrowserLoading = false
        }
    }

    public func loadRemoteFilePreview(path: String) {
        guard let handle = handle, state == .connected else {
            fileBrowserError = "Session is not connected"
            return
        }
        isFileBrowserLoading = true
        fileBrowserError = nil
        Task {
            do {
                self.filePreview = try await handle.readRemoteFilePreview(path: path, maxBytes: 131_072)
            } catch {
                self.fileBrowserError = error.localizedDescription
            }
            self.isFileBrowserLoading = false
        }
    }

    /// Types `cd '<path>'` + Enter into the active terminal.
    public func changeDirectory(to path: String) {
        executeAiCommand("cd " + quoteRemotePathForShell(path: path))
    }

    /// Inserts the shell-quoted path into the prompt without executing.
    public func insertRemotePath(_ path: String) {
        insertAiCommand(quoteRemotePathForShell(path: path) + " ")
    }

    /// Zero Battery Drain: gracefully closes the SSH TCP socket when iOS suspends the app.
    /// tmux session on remote MacBook persists.
    public func disconnectForBackground() {
        stopPortForward()
        guard let handle = handle else { return }
        print("[SwiftVM \(instanceId)] disconnectForBackground")
        handle.disconnect()
        self.state = .disconnected
    }

    /// Instant reconnection on app foreground (didBecomeActive).
    /// Instantly reconnects TCP and re-attaches to the active tmux session.
    public func reconnectOnForeground() {
        guard let handle = handle else { return }
        print("[SwiftVM \(instanceId)] reconnectOnForeground")
        do {
            try handle.reconnect()
        } catch {
            self.state = .failed(reason: error.localizedDescription)
        }
    }

    // MARK: - SshSessionCallback (invoked from Rust background worker thread)

    nonisolated public func onStateChanged(state: SessionState) {
        print("[SwiftVM \(instanceId)] onStateChanged: \(state)")
        Task { @MainActor in
            self.state = state
            if state == .connected {
                self.errorMessage = nil
                if self.pinnedHostKey == nil, let fp = self.handle?.hostKeyFingerprint() {
                    self.pinnedHostKey = fp
                    self.onHostKeyObserved?(fp)
                }
                if let handle = self.handle {
                    print("[SwiftVM \(self.instanceId)] Connected! Applying window size: \(self.lastCols)x\(self.lastRows)")
                    try? handle.resize(cols: self.lastCols, rows: self.lastRows)
                }
            }
        }
    }

    nonisolated public func onDataReceived(data: Data) {
        print("[SwiftVM \(instanceId)] onDataReceived: \(data.count) bytes")
        Task { @MainActor in
            self.terminalDataBuffer.append(data)
            if self.terminalDataBuffer.count > 1024 * 1024 {
                self.terminalDataBuffer.removeFirst(self.terminalDataBuffer.count - 1024 * 1024)
            }
            self.outputPublisher.send(data)
        }
    }

    nonisolated public func onError(message: String) {
        print("[SwiftVM \(instanceId)] onError: \(message)")
        Task { @MainActor in
            self.errorMessage = message
            self.state = .failed(reason: message)
        }
    }
}
