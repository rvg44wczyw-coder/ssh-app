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

    // Structured Transcript / Dual-Mode State
    @Published public var isChatMode: Bool = false
    private var watchedSum: String? = nil
    private var lastTranscriptDetect: Date?
    /// Set by the terminal view: pastes the clipboard honouring the program's bracketed-paste mode.
    public var pasteHandler: (() -> Void)?
    @Published public var transcriptTurns: [TranscriptTurn] = []
    @Published public var isTranscriptLoading: Bool = false
    @Published public var activeTranscriptPath: String? = nil
    @Published public var transcriptError: String? = nil
    /// The live line of the agent on the tmux screen ("✶ Thinking… (12s)"), when the feed is built from the screen.
    @Published public var screenStatus: String? = nil
    /// A question the agent waits on (permission, choice), read from the screen; shown as a card.
    @Published public var screenPrompt: ScreenPrompt? = nil
    /// What is typed in the agent's input box and not sent (read from the screen).
    @Published public var screenDraft: String? = nil
    /// What we believe the agent's box holds after our own typing (see `DraftSync`).
    private var draftInBox = ""
    /// Set while `sendImmediately` runs: the mirroring of the field stays out of its way.
    public private(set) var sendingNow = false
    private var draftUncertain = false
    private var draftGuardUntil = Date.distantPast
    /// The pane showed a known agent (not a shell) at the last look: only then is typing mirrored.
    private var feedRecognized = false
    private var lastScreenSum: String?
    private var lastScreenTurnCount = 0
    private var thinkingTurns: [TranscriptTurn] = []
    private var lastThinkingRead: Date?

    // Smart Snippets & Fuzzy History Search State
    @Published public var remoteHistoryCommands: [String] = []
    @Published public var isHistoryLoading: Bool = false

    // Host Telemetry & Process Monitor State
    @Published public var telemetrySnapshot: HostTelemetrySnapshot? = nil
    @Published public var isTelemetryLoading: Bool = false
    @Published public var telemetryError: String? = nil
    @Published public var telemetryUpdatedAt: Date? = nil

    // Remote File Browser State
    @Published public var fileListing: RemoteDirectoryListing? = nil
    @Published public var filePreview: RemoteFilePreview? = nil
    @Published public var isFileBrowserLoading: Bool = false
    /// Git state of the repository around the listed folder (nil outside a repository).
    @Published public var gitStatus: GitStatusResult? = nil
    @Published public var fileBrowserError: String? = nil

    /// System of the host (macOS, Linux, Windows via WSL2), detected once per connection.
    @Published public var hostTarget: HostTarget? = nil
    /// Public half of this tab's key, for the WSL2 setup snippets (the private key is not kept).
    public private(set) var publicKeyLine: String? = nil
    private var hostKey: String = ""
    /// `user@host:port`, the key the app uses for per-host settings (push registration).
    public var hostCacheKey: String { hostKey }

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

    /// Plain terminals start in the chosen terminal folder; agent tabs keep their profile's folder.
    private var startDir: String? {
        if let own = session.startDir { return own }
        guard session.command == nil, session.profileId == nil else { return nil }
        return TerminalFolder.normalized(TerminalFolder.stored)
    }

    public func start(host: String, port: UInt16, username: String, privateKeyPEM: String) {
        print("[SwiftVM \(instanceId)] start() host=\(host), port=\(port), user=\(username), state=\(state)")
        guard state != .connected && state != .connecting else {
            print("[SwiftVM \(instanceId)] start() skipped: already \(state)")
            return
        }

        self.terminalDataBuffer = Data()
        hostTarget = nil
        hostKey = HostTargetCache.key(host: host, port: port, username: username)
        publicKeyLine = try? derivePublicKey(privateKeyOpenssh: privateKeyPEM)

        // The first connection to a host in this app run asks which system it is (a Mac, Linux, WSL2
        // or native Windows), because that decides what keeps the session alive. It is advice only:
        // when it fails the tab uses tmux as before.
        if let known = HostTargetCache.target(forKey: hostKey) {
            hostTarget = known
            connect(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM,
                    multiplexer: hostTargetMultiplexer(target: known))
            return
        }
        self.state = .connecting
        self.errorMessage = nil
        let probe = RemoteServerConfig(host: host, port: port, username: username,
                                       privateKeyPem: privateKeyPEM, pinnedHostKey: pinnedHostKey)
        Task { @MainActor in
            let target = try? await detectHostTargetOnce(config: probe)
            var multiplexer = MultiplexerKind.tmux
            if let target = Self.conclusive(target) {
                hostTarget = target
                HostTargetCache.set(target, forKey: hostKey)
                multiplexer = hostTargetMultiplexer(target: target)
            }
            // On native Windows the helper IS the multiplexer: put the verified build in place first.
            if let target, hostTargetMultiplexer(target: target) == .windowsMux {
                do {
                    try await ensureWindowsHelperOnce(config: probe)
                } catch {
                    self.state = .failed(reason: "Could not install the Windows helper: \(error.localizedDescription)")
                    self.errorMessage = error.localizedDescription
                    return
                }
            }
            self.state = .disconnected
            connect(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM,
                    multiplexer: multiplexer)
        }
    }

    private func connect(host: String, port: UInt16, username: String, privateKeyPEM: String,
                         multiplexer: MultiplexerKind) {
        let config = SessionConfig(
            host: host,
            port: port,
            username: username,
            privateKeyPem: privateKeyPEM,
            sessionName: session.sessionName,
            initialCols: lastCols,
            initialRows: lastRows,
            command: session.command,
            startDir: startDir,
            pinnedHostKey: pinnedHostKey,
            multiplexer: multiplexer
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

    /// Size of the terminal as last reported by the view (used to lay out previews).
    public var terminalSize: (cols: Int, rows: Int) { (Int(lastCols), Int(lastRows)) }

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

    /// What runs in the tab's pane right now (AI CLI or shell), asked from the host every time and
    /// never cached: the user can start or leave an agent at any moment, and a stale "AI CLI" answer
    /// would send `!cmd` to a shell, where `!` is history expansion (it re-runs an earlier command).
    public func currentPaneContext() async -> PaneContext {
        guard let handle = handle, state == .connected else { return PaneContext(kind: .unknown, agent: "") }
        return (try? await handle.detectPaneContext(sessionName: session.sessionName))
            ?? PaneContext(kind: .unknown, agent: "")
    }

    /// Types a **shell** command: as it is in a shell, with a leading `!` inside an AI CLI
    /// (otherwise the agent would take it as a prompt). Agent slash commands (`/model`) are not
    /// shell commands and go through `insertAiCommand` / `executeAiCommand`.
    public func insertShellCommand(_ command: String) {
        Task {
            let context = await currentPaneContext()
            insertAiCommand(shellCommandForPane(context: context, command: command))
        }
    }

    public func runShellCommand(_ command: String) {
        Task {
            let context = await currentPaneContext()
            executeAiCommand(shellCommandForPane(context: context, command: command))
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
            lastScreenSum = nil
            if await reloadFromScreen() != nil {
                self.isTranscriptLoading = false
                return
            }
            do {
                // This tab's own agent, found from its tmux session; never "the newest on the host".
                let path = try await handle.detectSessionTranscriptPath(sessionName: session.sessionName)
                if path != self.activeTranscriptPath {
                    self.activeTranscriptPath = path
                    self.transcriptTurns = []
                }

                guard let validPath = path else {
                    self.transcriptError = "No agent (Claude Code or Antigravity) is running in this tab"
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

    /// Types the difference between the agent's input box and `text`, so the draft is kept on the
    /// host. `false`: not mirrored (no connection, not an agent, a question is up, too long).
    public func syncDraft(_ text: String) async -> Bool {
        guard handle != nil, state == .connected, feedRecognized, screenPrompt == nil,
              let bytes = DraftSync.bytes(from: draftInBox, to: text, uncertain: draftUncertain) else { return false }
        if !bytes.isEmpty { sendInput(Data(bytes)) }
        draftInBox = DraftSync.sanitized(text)
        draftUncertain = false
        return true
    }

    /// Interrupts the agent (one Esc, which both Claude Code and Antigravity take as "stop what you
    /// are doing") and sends `text` at once, instead of queueing it behind the running turn. The
    /// box is read again after the Esc (an agent may take its draft back), then filled and submitted.
    /// `false`: nothing sent (no connection, not an agent, a question is up, or the agent is idle,
    /// where a plain send is the same and a stray Esc could do harm).
    public func sendImmediately(_ text: String) async -> Bool {
        guard let handle, state == .connected, feedRecognized, screenPrompt == nil, screenStatus != nil else { return false }
        sendingNow = true
        defer { sendingNow = false }
        sendInput(Data([0x1b]))
        try? await Task.sleep(for: .milliseconds(450))
        var boxText = ""
        if let cap = try? await handle.captureSessionScreen(sessionName: session.sessionName, knownSum: nil),
           let screen = cap.text {
            let feed = parseScreenFeed(text: screen)
            if feed.prompt != nil { return false }
            boxText = feed.draft ?? ""
        }
        draftInBox = boxText
        draftUncertain = !boxText.isEmpty
        guard await syncDraft(text) else { return false }
        try? await Task.sleep(for: .milliseconds(120))
        sendInput(Data([0x0d]))
        draftWasSent()
        return true
    }

    /// The box was submitted: it is empty now.
    public func draftWasSent() {
        draftInBox = ""
        draftUncertain = false
        screenDraft = nil
        draftGuardUntil = Date().addingTimeInterval(4)
    }

    /// The draft on the screen when we have typed nothing ourselves and the field is empty.
    public func adoptableDraft(current: String) -> String? {
        guard current.isEmpty, draftInBox.isEmpty, Date() >= draftGuardUntil,
              let d = screenDraft, !d.isEmpty else { return nil }
        draftInBox = d
        draftUncertain = true
        return d
    }

    /// Looks at the pane only for a question the agent waits on (the banner's watcher, for tabs whose
    /// feed is not open). Its own checksum: it never disturbs the feed's reading.
    public func watchPromptOnce() async {
        guard let handle = handle, state == .connected else { return }
        guard let cap = try? await handle.captureSessionScreen(sessionName: session.sessionName, knownSum: watchedSum),
              let text = cap.text else { return }
        watchedSum = cap.checksum
        let feed = parseScreenFeed(text: text)
        let prompt = feed.dialect == "unknown" ? nil : feed.prompt
        if screenPrompt != prompt { screenPrompt = prompt }
    }

    /// Builds the feed from the tmux pane. `nil`: nothing recognised on the screen (a plain shell, an
    /// unknown program, no tmux), the caller falls back to the session file. `true` / `false`: the
    /// turns shown changed / did not.
    private func reloadFromScreen() async -> Bool? {
        guard let handle = handle, state == .connected else { return nil }
        guard let first = try? await handle.captureSessionScreen(sessionName: session.sessionName, knownSum: lastScreenSum)
        else { return nil }
        guard var text = first.text else { return false }   // the same screen as before
        // The agent redraws its last lines while it writes (spinner, a result coming in, a reply
        // being typed): a frame taken in the middle is the "mess" at the end of the feed. Wait
        // until two captures in a row are the same (at most three more looks, ~1 s).
        var cap = first
        for _ in 0..<3 {
            try? await Task.sleep(for: .milliseconds(350))
            guard let again = try? await handle.captureSessionScreen(sessionName: session.sessionName, knownSum: cap.checksum),
                  let newer = again.text else { break }   // unchanged: the frame is stable
            cap = again
            text = newer
        }
        let feed = parseScreenFeed(text: text)
        feedRecognized = feed.dialect != "unknown"
        guard feed.dialect != "unknown" else { lastScreenSum = nil; return nil }
        lastScreenSum = cap.checksum
        if screenDraft != feed.draft { screenDraft = feed.draft }
        if screenStatus != feed.status { screenStatus = feed.status }
        if screenPrompt != feed.prompt { screenPrompt = feed.prompt }
        var turns = feed.turns
        // Claude's screen has no thinking: it comes from the session file, in its place. Both
        // agents fold groups of calls ("Ran 5 commands (ctrl+o to expand)") and cut results; the
        // file has them whole. agy shows its thinking on screen, so only the calls are taken.
        let grew = feed.turns.count != lastScreenTurnCount
        var file = await fileTurnsForThinking(handle, force: grew, screen: text)
        if feed.dialect == "agy" {
            for i in file.indices { file[i].thinking = nil }
        }
        turns = mergeThinking(screen: turns, file: file)
        lastScreenTurnCount = feed.turns.count
        transcriptError = nil
        guard turns != transcriptTurns else { return false }
        transcriptTurns = turns
        return true
    }

    /// Answers the menu on the screen from the card: the cursor goes to the option with the arrow
    /// keys, then Enter (a choice) or Space (a box). Enter is its own write: after a burst of keys
    /// an agent's menu would take it for part of one input.
    public func answerPrompt(index: Int, toggle: Bool) {
        guard let p = screenPrompt, index >= 0, index < p.options.count else { return }
        let delta = index - Int(p.selected)
        var keys = Data()
        let arrow = delta > 0 ? "\u{1b}[B" : "\u{1b}[A"
        for _ in 0..<abs(delta) { keys.append(Data(arrow.utf8)) }
        if toggle { keys.append(Data(" ".utf8)) }
        if !keys.isEmpty { sendInput(keys) }
        if !toggle {
            Task {
                try? await Task.sleep(for: .milliseconds(120))
                self.sendInput(Data([0x0d]))
                try? await Task.sleep(for: .milliseconds(400))
                self.lastScreenSum = nil
                _ = await self.reloadFromScreen()
            }
        } else {
            Task {
                try? await Task.sleep(for: .milliseconds(300))
                self.lastScreenSum = nil
                _ = await self.reloadFromScreen()
            }
        }
    }

    /// Restarts the agent of this tab in its tmux session with `command`. The terminal stays attached,
    /// the feed starts over from the new screen.
    public func restartAgent(command: String) async throws {
        guard let handle, state == .connected else { throw SshCoreError.NotConnected }
        try await handle.restartSessionCommand(sessionName: session.sessionName, command: command)
        lastScreenSum = nil
        watchedSum = nil
        screenPrompt = nil
        screenStatus = nil
        transcriptTurns = []
        thinkingTurns = []
        try? await Task.sleep(for: .seconds(2))
        _ = await reloadFromScreen()
    }

    /// Sends a picked file to the host's inbox; returns its path there.
    /// `permanent`: kept in the host's `~/.ssh-app/inbox`; otherwise in its temp folder.
    public func uploadAttachment(_ url: URL, name: String, permanent: Bool) async throws -> String {
        guard let handle, state == .connected else { throw SshCoreError.NotConnected }
        return try await handle.uploadAttachment(srcLocalPath: url.path, fileName: name, permanent: permanent)
    }

    /// Moves between the tabs of a multi-question form (Left / Right), so earlier answers can be changed.
    public func moveTab(back: Bool) {
        sendInput(Data([0x1b, 0x5b, back ? 0x44 : 0x43]))
        Task {
            try? await Task.sleep(for: .milliseconds(400))
            self.lastScreenSum = nil
            _ = await self.reloadFromScreen()
        }
    }

    /// Confirms a multi-choice menu (Enter) or cancels the question (Esc).
    public func finishPrompt(confirm: Bool) {
        sendInput(Data([confirm ? 0x0d : 0x1b]))
        Task {
            try? await Task.sleep(for: .milliseconds(400))
            self.lastScreenSum = nil
            _ = await self.reloadFromScreen()
        }
    }

    private func fileTurnsForThinking(_ handle: SshSessionHandle, force: Bool, screen: String) async -> [TranscriptTurn] {
        // New turns on the screen: read the file now, so their thinking appears with them and does
        // not jump in later (and shift the text) when the 15 s refresh comes.
        if !force, let last = lastThinkingRead, Date().timeIntervalSince(last) < 15 { return thinkingTurns }
        lastThinkingRead = Date()
        if let found = try? await handle.detectSessionTranscriptPath(sessionName: session.sessionName),
           let turns = try? await handle.readRemoteTranscript(remotePath: found) {
            // The same agent may also run in this folder elsewhere (the laptop): a file whose
            // conversation is not the one on the screen is not used.
            thinkingTurns = transcriptMatchesScreen(file: turns, screen: screen) ? turns : []
        }
        return thinkingTurns
    }

    /// Live tail for the feed: re-reads the transcript without touching the loading or error state,
    /// and publishes only when something changed (so the view does not re-render or scroll).
    /// A failure keeps what is on screen.
    /// Returns whether the shown turns changed (the caller slows the polling down while nothing happens).
    @discardableResult
    public func reloadTranscriptQuietly() async -> Bool {
        guard let handle = handle, state == .connected, !isTranscriptLoading else { return false }
        // The feed is the tmux screen (full history while the agent draws in the normal buffer); the
        // session file is only the fallback and the source of the thinking.
        if let changed = await reloadFromScreen() { return changed }
        // Which transcript belongs to this tab can change (the agent was quit or another one
        // started), so ask the host again, but not more often than every 20 s.
        if lastTranscriptDetect == nil || Date().timeIntervalSince(lastTranscriptDetect!) >= 20 {
            lastTranscriptDetect = Date()
            // A failed lookup (outer nil) keeps what is shown; "no agent" (inner nil) clears it.
            if let found = try? await handle.detectSessionTranscriptPath(sessionName: session.sessionName),
               found != activeTranscriptPath {
                activeTranscriptPath = found
                transcriptTurns = []
            }
        }
        guard let path = activeTranscriptPath, let turns = try? await handle.readRemoteTranscript(remotePath: path) else { return false }
        var changed = false
        if turns != transcriptTurns { transcriptTurns = turns; changed = true }
        if transcriptError != nil, !turns.isEmpty { transcriptError = nil }
        return changed
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

    /// A probe that got no answer (slow login shell, dropped line) says nothing about the system: it
    /// is not remembered, so the next look asks again, instead of the card calling a Mac "Windows".
    private static func conclusive(_ target: HostTarget?) -> HostTarget? {
        if case .unsupported(let reason)? = target, reason.hasPrefix("The host did not answer") { return nil }
        return target
    }

    /// SHA-256 of the helper build that fits this host. Until the system is known (or when it cannot
    /// be told) it is the Mac build's, which keeps the behaviour of a Mac host unchanged.
    public func helperPin() async -> String? {
        if hostTarget == nil, let handle, state == .connected {
            hostTarget = Self.conclusive(try? await handle.detectHostTarget())
            if let target = hostTarget { HostTargetCache.set(target, forKey: hostKey) }
        }
        guard let target = hostTarget else { return bundledHostHelperSha256() }
        return bundledHostHelperSha256For(target: target)
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
                let snap = try await handle.fetchHostTelemetry(helperSha256: await helperPin())
                self.telemetrySnapshot = snap
                self.telemetryUpdatedAt = Date()
                self.isTelemetryLoading = false
            } catch {
                self.telemetryError = error.localizedDescription
                self.isTelemetryLoading = false
            }
        }
    }

    public func killRemoteProcess(pid: UInt32, force: Bool, started: UInt64? = nil) {
        guard let handle = handle, state == .connected else { return }
        Task {
            do {
                try await handle.killRemoteProcess(pid: pid, force: force, started: started, helperSha256: await helperPin())
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
                let listing = try await handle.listRemoteDirectory(path: path, includeHidden: includeHidden, helperSha256: await helperPin())
                self.fileListing = listing
                // Git colours come second: the list is shown first and a slow or failing git never blocks it.
                let status = try? await handle.fetchGitStatus(dir: listing.currentPath)
                self.gitStatus = (status?.isRepo == true) ? status : nil
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
                self.filePreview = try await handle.readRemoteFilePreview(path: path, maxBytes: 131_072, helperSha256: await helperPin())
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
                // Keeps the push registration of hosts the user registered once current (token changes).
                Task { await PushManager.shared.syncIfRegistered(viewModel: self) }
            }
        }
    }

    nonisolated public func onDataReceived(data: Data) {
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
