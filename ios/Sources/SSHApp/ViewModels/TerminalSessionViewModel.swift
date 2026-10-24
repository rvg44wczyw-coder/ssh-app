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
    /// What keeps this tab alive on the host. Zellij tabs are terminal only: the agent feed and
    /// everything else that reads the screen or sends keys needs tmux.
    @Published public private(set) var multiplexer: MultiplexerKind = .tmux
    /// Counted up by the session menu's "Find in feed"; the feed opens its search when it changes.
    @Published public var feedFindRequests = 0
    private var watchedSum: String? = nil
    private var lastTranscriptDetect: Date?
    /// Set by the terminal view: pastes the clipboard honouring the program's bracketed-paste mode.
    public var pasteHandler: (() -> Void)?
    /// Same, for text the system paste button handed over (no pasteboard read, so no "Allow Paste" prompt).
    public var pasteTextHandler: ((String) -> Void)?
    @Published public var transcriptTurns: [TranscriptTurn] = [] {
        didSet { if transcriptTurns != oldValue { recountUnread() } }
    }
    /// Messages of the agent to the user (text answers) written after the chat was last open.
    /// Service entries, tool calls and shell output are not counted.
    @Published public private(set) var unreadCount = 0
    private var isViewed = false
    private var lastOpenedAt: Date {
        get {
            let t = UserDefaults.standard.double(forKey: "chat_last_opened_" + session.id.uuidString)
            if t > 0 { return Date(timeIntervalSince1970: t) }
            // First sight of this chat: what is already there is not new.
            let now = Date()
            UserDefaults.standard.set(now.timeIntervalSince1970, forKey: "chat_last_opened_" + session.id.uuidString)
            return now
        }
        set { UserDefaults.standard.set(newValue.timeIntervalSince1970, forKey: "chat_last_opened_" + session.id.uuidString) }
    }
    @Published public var isTranscriptLoading: Bool = false
    @Published public var activeTranscriptPath: String? = nil
    @Published public var transcriptError: String? = nil
    /// The live line of the agent on the tmux screen ("✶ Thinking… (12s)"), when the feed is built from the screen.
    @Published public var screenStatus: String? = nil
    /// Messages sent from the phone that the session file does not show yet (the agent queues them
    /// while it works, and even an idle one writes them a moment later): shown at the end of the feed so a
    /// sent message never seems to vanish.
    @Published public private(set) var pendingEchoes: [PendingEcho] = []

    public struct PendingEcho: Identifiable, Equatable {
        public let id = "echo_" + UUID().uuidString
        let text: String
        /// How many of your messages with this exact text the file had when it was sent.
        let baseCount: Int
        let sentAt = Date()
    }

    /// A message of ours that sits in the working agent's queue; offered for "send now" until the agent is idle.
    @Published public var queuedNote: String? = nil
    /// How long the last feed refresh took (shown in the status line when "Show feed timing" is on).
    @Published public private(set) var feedTiming: FeedTiming?
    private var didReadFile = false

    public struct FeedTiming: Equatable {
        public let captureMs: Int
        public let fileMs: Int?
        public let cycleMs: Int
        public let fileTurns: Int
        public let at: Date
    }

    private static func millis(since start: ContinuousClock.Instant) -> Int {
        let d = ContinuousClock.now - start
        return Int(d.components.seconds * 1000 + d.components.attoseconds / 1_000_000_000_000_000)
    }
    /// A question the agent waits on (permission, choice), read from the screen; shown as a card.
    @Published public var screenPrompt: ScreenPrompt? = nil
    /// The `/btw` overlay and the dialog of a slash command, as the pane shows them right now.
    @Published public var screenAside: ScreenAside? = nil
    @Published public var screenDialog: ScreenDialog? = nil
    /// What is typed in the agent's input box and not sent (read from the screen).
    @Published public var screenDraft: String? = nil
    /// Repository, branch and worktree of the tab's folder, for the feed header.
    @Published public var gitContext: GitSessionContext = .none
    /// Files with uncommitted changes in that repository (0 when clean or not a repository).
    @Published public var gitDirtyCount = 0
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
    /// A file opened from a mention in the feed: the sheet shows `filePreview` at `line`.
    @Published public var fileReference: FileReferenceTarget? = nil
    /// Repository root of the tab (from the git probe), the first place a relative path is looked for.
    private var gitRepoRoot: String?
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
    /// The session file kept as turns and the place read to (see `syncFileTurns`).
    private let fileFeed = FeedState()
    private var fileCursor: FeedCursor?
    private var fileFeedPath: String?
    /// `fileFeed` holds turns read from the agent's own database (Antigravity), not from its file.
    private var feedFromDatabase = false
    private var screenIsAgy = false
    /// The agent in this tab speaks Antigravity's commands (a tab made for it, or one it was started in by hand).
    public var isAgyAgent: Bool { session.agentType == .antigravity || screenIsAgy }
    /// The host pushes changes of the session file and of the pane's tail over one open channel
    /// (`watch_feed`); while it runs nothing is polled.
    private var feedWatch: FeedWatch?
    @Published public private(set) var feedStreaming = false
    private var streamReloadPending = false
    private var lastStreamAttempt: Date?
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
        self.isChatMode = Self.startsInFeed(session)
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
            let base = hostTargetMultiplexer(target: known)
            if base != .tmux {
                connect(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM, multiplexer: base)
                return
            }
            self.state = .connecting
            Task { @MainActor in
                let mux = await chooseMultiplexer(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM)
                self.state = .disconnected
                connect(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM, multiplexer: mux)
            }
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
            if multiplexer == .tmux {
                multiplexer = await chooseMultiplexer(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM)
            }
            self.state = .disconnected
            connect(host: host, port: port, username: username, privateKeyPEM: privateKeyPEM,
                    multiplexer: multiplexer)
        }
    }

    /// tmux unless the server prefers Zellij (and it is there) or tmux is not installed at all.
    /// One short connection per host and app run; any failure means tmux, as before.
    private func chooseMultiplexer(host: String, port: UInt16, username: String, privateKeyPEM: String) async -> MultiplexerKind {
        if let known = HostTargetCache.chosenMultiplexer(forKey: hostKey) { return known }
        let probe = RemoteServerConfig(host: host, port: port, username: username,
                                       privateKeyPem: privateKeyPEM, pinnedHostKey: pinnedHostKey)
        let pref = HostTargetCache.multiplexerPreference(forKey: hostKey)
        let kind = (try? await detectMultiplexerOnce(config: probe, preference: pref)) ?? .tmux
        HostTargetCache.setChosenMultiplexer(kind, forKey: hostKey)
        return kind
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
        self.multiplexer = multiplexer
        // The feed reads the tmux pane: a Zellij tab is terminal only.
        if multiplexer != .tmux, isChatMode { isChatMode = false }

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
        return (try? await handle.detectPaneContextIn(sessionName: session.sessionName, kind: multiplexer))
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

    /// The feed is the default view of an agent tab; a plain shell or container shell starts as a
    /// terminal. What the owner picked for this one session (the feed/terminal switch) wins.
    private static func startsInFeed(_ session: TabSession) -> Bool {
        if let picked = UserDefaults.standard.object(forKey: "feed_mode_" + session.id.uuidString) as? Bool { return picked }
        switch session.agentType {
        case .claudeCode, .antigravity, .localLlm: return true
        case .customShell, .dockerContainer: return false
        }
    }

    public func toggleChatMode() {
        isChatMode.toggle()
        UserDefaults.standard.set(isChatMode, forKey: "feed_mode_" + session.id.uuidString)
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
                let path = try await handle.detectSessionTranscriptPathIn(sessionName: session.sessionName, kind: multiplexer)
                if path != self.activeTranscriptPath {
                    self.activeTranscriptPath = path
                    self.transcriptTurns = []
                }

                guard let validPath = path else {
                    self.transcriptError = "No agent (Claude Code or Antigravity) is running in this tab"
                    self.isTranscriptLoading = false
                    return
                }

                guard let turns = await self.syncFileTurns(handle, validPath, fresh: true) else {
                    throw SshCoreError.TranscriptError(reason: "Transcript file not found")
                }
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

    /// Name of the agent in this tab for the feed header.
    public var feedAgentName: String {
        switch session.agentType {
        case .claudeCode: return "Claude Code"
        case .antigravity: return "Antigravity"
        default: return session.title
        }
    }

    /// Asks the host which repository, branch and worktree the tab is in (once per call; failures keep
    /// what is shown).
    public func loadGitContext() async {
        guard let handle, state == .connected,
              let result = try? await handle.fetchGitWorktrees(sessionName: session.sessionName) else { return }
        let next = GitSessionContext(result)
        gitRepoRoot = result.status == .ok ? result.repoRoot : nil
        if next != gitContext { gitContext = next }
        if next.isGit, let changes = try? await handle.fetchGitChanges(sessionName: session.sessionName) {
            let n = changes.status == .ok ? changes.entries.count : 0
            if n != gitDirtyCount { gitDirtyCount = n }
        } else if gitDirtyCount != 0 {
            gitDirtyCount = 0
        }
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
        if let cap = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: nil, kind: multiplexer),
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

    /// A send is under way: the strip that offers the agent's box stays away, so the text just sent
    /// is not shown again for the moment before the screen catches up.
    public func draftIsBeingSent() {
        draftGuardUntil = Date().addingTimeInterval(4)
    }

    private func userCount(of text: String) -> Int {
        // A message the agent took from its queue is written as a note ("Message sent while working")
        // when the file does not mark it as typed by a person, so notes count too.
        transcriptTurns.filter { ($0.turnType == .user || $0.turnType == .system) && ($0.content?.contains(text) ?? false) }.count
    }

    /// Remembers a message just sent so the feed can show it before the file does.
    public func echoSent(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        pendingEchoes.append(PendingEcho(text: t, baseCount: userCount(of: t)))
        if pendingEchoes.count > 4 { pendingEchoes.removeFirst() }
    }

    /// The transcript with the not yet delivered messages at its end. An echo goes when the file shows
    /// the message, or when the agent is idle and the message is old (it was lost or edited).
    public var turnsWithEchoes: [TranscriptTurn] {
        guard !pendingEchoes.isEmpty else { return transcriptTurns }
        var out = transcriptTurns
        var step = (transcriptTurns.last?.stepIndex ?? 0) + 1
        for e in pendingEchoes {
            if userCount(of: e.text) > e.baseCount { continue }
            if screenStatus == nil, Date().timeIntervalSince(e.sentAt) > 30 { continue }
            out.append(TranscriptTurn(id: e.id, stepIndex: step, turnType: .user, timestamp: nil, content: e.text,
                                      thinking: nil, toolCalls: [], status: .done, label: nil))
            step += 1
        }
        return out
    }

    /// Drops echoes the file has shown already (called when the transcript changes).
    public func pruneEchoes() {
        let kept = pendingEchoes.filter { userCount(of: $0.text) <= $0.baseCount && Date().timeIntervalSince($0.sentAt) < 600 }
        if kept != pendingEchoes { pendingEchoes = kept }
    }

    /// What the agent's input box holds right now, read from the pane (`nil`: could not be read).
    public func currentBoxDraft() async -> String? {
        guard let handle, state == .connected,
              let cap = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: nil, kind: multiplexer),
              let screen = cap.text else { return nil }
        return parseScreenFeed(text: screen).draft ?? ""
    }

    /// Presses Enter until the box is empty (an Enter right after typed text can be taken as part of
    /// it, and the agent may be busy for a moment); at most three presses.
    ///
    /// With `acceptSuggestion` Tab goes first: Claude's own suggestion for the next prompt is only a
    /// grey hint that Enter does not send; Tab turns it into real text (on real text it does nothing).
    public func submitBox(acceptSuggestion: Bool = false) async {
        if acceptSuggestion {
            sendInput(Data([0x09]))
            try? await Task.sleep(for: .milliseconds(250))
        }
        for attempt in 0..<3 {
            sendInput(Data([0x0d]))
            try? await Task.sleep(for: .milliseconds(700))
            guard let left = await currentBoxDraft(), !left.isEmpty, attempt < 2 else { return }
        }
    }

    /// The agent's own "send now" for a queued message (Ctrl+X, Ctrl+S): it stops the turn and the
    /// queued message goes first.
    public func sendQueuedNow() {
        sendInput(Data([0x18, 0x13]))
    }

    /// The box was submitted: it is empty now.
    public func draftWasSent() {
        draftInBox = ""
        draftUncertain = false
        screenDraft = nil
        draftGuardUntil = Date().addingTimeInterval(4)
    }

    /// What the strip above the field may offer: nothing in the seconds after a send (the agent's
    /// box is still redrawing, or shows its own queue hint), nor its hints.
    public var offerableScreenDraft: String? {
        guard Date() >= draftGuardUntil, let d = screenDraft, !d.isEmpty,
              !d.lowercased().contains("to edit queued messages") else { return nil }
        return d
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
    /// The chat is on screen (`true`) or left (`false`). Opening and leaving both stamp the time of
    /// the last opening, so what arrived while it was open is not counted as unread afterwards.
    public func setViewed(_ viewed: Bool) {
        guard viewed != isViewed || viewed else { return }
        isViewed = viewed
        lastOpenedAt = Date()
        if unreadCount != 0 { unreadCount = 0 }
    }

    private func recountUnread() {
        guard !isViewed else { return }
        let since = lastOpenedAt
        let n = transcriptTurns.filter { turn in
            guard turn.turnType == .assistant,
                  turn.content?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false,
                  let stamp = turn.timestamp, let at = Self.parseStamp(stamp) else { return false }
            return at > since
        }.count
        if n != unreadCount { unreadCount = n }
    }

    private static func parseStamp(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: s) { return d }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: s)
    }

    /// For a tab that is not on screen: reads the session file now and then so the unread count
    /// follows the agent. Does nothing while the chat is open (the feed has its own polling).
    public func refreshUnreadOnce() async {
        guard !isViewed, let handle = handle, state == .connected else { return }
        if activeTranscriptPath == nil,
           let found = try? await handle.detectSessionTranscriptPathIn(sessionName: session.sessionName, kind: multiplexer) {
            activeTranscriptPath = found
        }
        guard let path = activeTranscriptPath,
              let turns = await syncFileTurns(handle, path) else { return }
        if turns != transcriptTurns { transcriptTurns = turns }
    }

    /// Types a slash command of the agent at its prompt (the line is cleared first); its dialog
    /// then shows up on the next screen read.
    public func runAgentCommand(_ command: String) {
        guard command.hasPrefix("/"), command.allSatisfy({ $0.isLetter || $0 == "/" || $0 == "-" }) else { return }
        sendInput(Data([0x15]) + Data(command.utf8) + Data([0x0d]))
        Task {
            try? await Task.sleep(for: .milliseconds(600))
            _ = await reloadTranscriptQuietly()
        }
    }

    /// The subagents of this tab's Claude session (newest first); `nil` when the host could not be
    /// asked or the tab has no session file. Nothing runs in the background: the list is read when
    /// the subagents screen opens or is refreshed.
    public func fetchSubagents() async -> [SubagentInfo]? {
        guard let handle, state == .connected else { return nil }
        if screenIsAgy {
            // Antigravity: child conversations from its index, through the pinned host helper.
            guard let pin = await helperPin() else { return nil }
            return try? await handle.listAgySubagents(sessionName: session.sessionName, helperSha256: pin)
        }
        guard let file = activeTranscriptPath ?? fileFeedPath, file.hasSuffix(".jsonl") else { return nil }
        return try? await handle.listSubagents(sessionFile: file)
    }

    /// What one subagent's file (or child conversation) gained since `cursor`, read into
    /// `FeedState.newSubagent()`. Outer `nil`: the host could not be asked; inner `nil`: no such file.
    public func readSubagent(_ agentId: String, cursor: FeedCursor?) async -> FeedRead?? {
        guard let handle, state == .connected else { return nil }
        do {
            if screenIsAgy {
                guard let pin = await helperPin() else { return nil }
                return .some(try await handle.readAgySubagentSince(conversationId: agentId, helperSha256: pin, cursor: cursor))
            }
            guard let file = activeTranscriptPath ?? fileFeedPath else { return nil }
            return .some(try await handle.readSubagentSince(sessionFile: file, agentId: agentId, cursor: cursor))
        } catch {
            return nil
        }
    }

    /// TCP ports the host listens on; `nil` when the host could not be asked.
    public func fetchListeningPorts() async -> [ListeningPort]? {
        guard let handle, state == .connected else { return nil }
        return try? await handle.fetchListeningPorts()
    }

    /// The session file as turns, read by cursor: after the first read only the new lines travel and
    /// are added to `fileFeed`; a replaced or shortened file starts again. `fresh` forgets the cursor.
    private func syncFileTurns(_ handle: SshSessionHandle, _ path: String, fresh: Bool = false) async -> [TranscriptTurn]? {
        // The stream feeds `fileFeed` itself: reading too would add the same lines twice.
        if feedStreaming, !fresh, fileFeedPath == path { return fileFeed.turns() }
        if fresh || fileFeedPath != path || feedFromDatabase {
            feedFromDatabase = false
            fileFeed.reset()
            fileCursor = nil
            fileFeedPath = path
        }
        guard let read = try? await handle.readRemoteTranscriptSince(remotePath: path, cursor: fileCursor) else { return nil }
        if read.reset { fileFeed.reset() }
        _ = fileFeed.applyChunk(chunk: read.text)
        fileCursor = read.cursor
        return fileFeed.turns()
    }

    /// Starts the push stream of this tab's feed when it can run: tmux, a connected session and an
    /// agent whose session file is known. Safe to call often; it restarts when the file changed.
    public func ensureFeedStream() async {
        guard let handle, state == .connected, multiplexer == .tmux else { return }
        // Looking for the agent's file walks the host's processes: not more often than every 20 s.
        if let last = lastStreamAttempt, Date().timeIntervalSince(last) < 20 { return }
        lastStreamAttempt = Date()
        guard let found = try? await handle.detectSessionTranscriptPathIn(sessionName: session.sessionName, kind: multiplexer) else { return }
        if feedStreaming, fileFeedPath == found { return }
        stopFeedStream()
        if fileFeedPath != found {
            fileFeed.reset()
            fileCursor = nil
            fileFeedPath = found
        }
        let bridge = FeedStreamBridge(self)
        // Antigravity keeps its conversation in a database the pinned host helper reads; without the
        // helper (or a conversation yet) that stream ends at once and the file is read as before.
        let fromDatabase = screenIsAgy
        if fromDatabase != feedFromDatabase {
            fileFeed.reset()
            fileCursor = nil
            feedFromDatabase = fromDatabase
        }
        do {
            if fromDatabase {
                guard let pin = await helperPin() else { feedFromDatabase = false; return }
                feedWatch = try await handle.watchAgyFeed(sessionName: session.sessionName, helperSha256: pin, state: fileFeed, cursor: fileCursor, listener: bridge)
            } else {
                feedWatch = try await handle.watchFeed(sessionName: session.sessionName, remotePath: found, state: fileFeed, cursor: fileCursor, listener: bridge)
            }
            feedStreaming = true
        } catch {
            feedStreaming = false
        }
    }

    public func stopFeedStream() {
        lastStreamAttempt = nil
        feedWatch?.stop()
        feedWatch = nil
        feedStreaming = false
    }

    fileprivate func streamChanged(_ cursor: FeedCursor) {
        fileCursor = cursor
        scheduleStreamReload()
    }

    /// An event arrived: read the screen once (coalescing bursts), no timer involved.
    fileprivate func scheduleStreamReload() {
        guard !streamReloadPending else { return }
        streamReloadPending = true
        Task {
            try? await Task.sleep(for: .milliseconds(150))
            streamReloadPending = false
            _ = await reloadTranscriptQuietly()
        }
    }

    fileprivate func streamClosed() {
        feedWatch = nil
        feedStreaming = false
    }

    /// Which container runtimes (and Kubernetes) the host has; `nil` when the host could not be asked.
    public func probeRuntimes() async -> [ContainerRuntime]? {
        guard let handle, state == .connected else { return nil }
        return try? await handle.probeContainerRuntimes()
    }

    /// Runs Claude's `/context` in the pane and reads the report from the screen. `nil`: the agent is
    /// busy (its input is not free), not connected, or the report did not appear in time.
    public func measureContext() async -> ContextUsage? {
        guard let handle, state == .connected, screenStatus == nil, screenPrompt == nil, screenDialog == nil else { return nil }
        runAgentCommand("/context")
        for _ in 0..<6 {
            try? await Task.sleep(for: .milliseconds(700))
            guard let cap = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: nil, kind: multiplexer),
                  let text = cap.text else { continue }
            if let usage = parseContextUsage(text: text), usage.windowTokens > 0 { return usage }
        }
        return nil
    }

    /// Fixed keys for a dialog (never text of a row); the screen is read again afterwards.
    public func sendDialogKeys(_ keys: [[UInt8]]) {
        for k in keys { sendInput(Data(k)) }
        Task {
            try? await Task.sleep(for: .milliseconds(500))
            _ = await reloadTranscriptQuietly()
        }
    }

    /// Leaves a dialog or the `/btw` overlay (Esc) and forgets it here at once.
    public func closeOverlay() {
        sendInput(Data([0x1b]))
        screenDialog = nil
        screenAside = nil
        Task {
            try? await Task.sleep(for: .milliseconds(500))
            _ = await reloadTranscriptQuietly()
        }
    }

    public func watchPromptOnce() async {
        guard let handle = handle, state == .connected else { return }
        guard let cap = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: watchedSum, kind: multiplexer),
              let text = cap.text else { return }
        watchedSum = cap.checksum
        let feed = parseScreenFeed(text: text)
        let prompt = feed.dialect == .unknown ? nil : feed.prompt
        if screenPrompt != prompt { screenPrompt = prompt }
    }

    /// Builds the feed from the tmux pane. `nil`: nothing recognised on the screen (a plain shell, an
    /// unknown program, no tmux), the caller falls back to the session file. `true` / `false`: the
    /// turns shown changed / did not.
    private func reloadFromScreen() async -> Bool? {
        guard let handle = handle, state == .connected else { return nil }
        let cycleStart = ContinuousClock.now
        guard let first = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: lastScreenSum, kind: multiplexer)
        else { return nil }
        let captureMs = Self.millis(since: cycleStart)
        guard var text = first.text else { return false }   // the same screen as before
        // The agent redraws its last lines while it writes (spinner, a result coming in, a reply
        // being typed). With tmux the host waits for a stable frame itself and sends it once;
        // only Zellij's capture still needs the looks here (until two in a row are the same).
        var cap = first
        if multiplexer != .tmux {
            for _ in 0..<3 {
                try? await Task.sleep(for: .milliseconds(350))
                guard let again = try? await handle.captureSessionScreenIn(sessionName: session.sessionName, knownSum: cap.checksum, kind: multiplexer),
                      let newer = again.text else { break }   // unchanged: the frame is stable
                cap = again
                text = newer
            }
        }
        let feed = parseScreenFeed(text: text)
        feedRecognized = feed.dialect != .unknown
        screenIsAgy = feed.dialect == .agy
        guard feed.dialect != .unknown else { lastScreenSum = nil; return nil }
        lastScreenSum = cap.checksum
        if screenDraft != feed.draft { screenDraft = feed.draft }
        if screenStatus != feed.status { screenStatus = feed.status }
        if feed.status == nil, queuedNote != nil { queuedNote = nil }
        if screenPrompt != feed.prompt { screenPrompt = feed.prompt }
        if screenAside != feed.aside { screenAside = feed.aside }
        if screenDialog != feed.dialog { screenDialog = feed.dialog }
        var turns = feed.turns
        // Claude's screen has no thinking: it comes from the session file, in its place. Both
        // agents fold groups of calls ("Ran 5 commands (ctrl+o to expand)") and cut results; the
        // file has them whole. agy shows its thinking on screen, so only the calls are taken.
        let grew = feed.turns.count != lastScreenTurnCount
        let fileStart = ContinuousClock.now
        var file = await fileTurnsForThinking(handle, force: grew, screen: text)
        let fileMs = didReadFile ? Self.millis(since: fileStart) : nil
        didReadFile = false
        if feed.dialect == .agy {
            for i in file.indices { file[i].thinking = nil }
        }
        turns = mergeThinking(screen: turns, file: file)
        lastScreenTurnCount = feed.turns.count
        feedTiming = FeedTiming(captureMs: captureMs, fileMs: fileMs, cycleMs: Self.millis(since: cycleStart),
                                fileTurns: file.count, at: Date())
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

    /// Answers a free-text option ("Type something."): the cursor goes to it, the text is typed and
    /// sent with Enter as its own write. Newlines become spaces: Enter would submit.
    public func answerPrompt(index: Int, text: String) {
        guard let p = screenPrompt, index >= 0, index < p.options.count else { return }
        let delta = index - Int(p.selected)
        var keys = Data()
        let arrow = delta > 0 ? "\u{1b}[B" : "\u{1b}[A"
        for _ in 0..<abs(delta) { keys.append(Data(arrow.utf8)) }
        if !keys.isEmpty { sendInput(keys) }
        let line = text.replacingOccurrences(of: "\n", with: " ")
        Task {
            try? await Task.sleep(for: .milliseconds(200))
            self.sendInput(Data(line.utf8))
            try? await Task.sleep(for: .milliseconds(250))
            self.sendInput(Data([0x0d]))
            try? await Task.sleep(for: .milliseconds(500))
            self.lastScreenSum = nil
            _ = await self.reloadFromScreen()
        }
    }

    /// Restarts the agent of this tab in its tmux session with `command`. The terminal stays attached,
    /// the feed starts over from the new screen.
    public func restartAgent(command: String) async throws {
        guard let handle, state == .connected else { throw SshCoreError.NotConnected }
        try await handle.restartSessionCommandIn(sessionName: session.sessionName, command: command, kind: multiplexer)
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
    public func uploadAttachment(_ url: URL, name: String, permanent: Bool, uploadId: String) async throws -> String {
        guard let handle, state == .connected else { throw SshCoreError.NotConnected }
        return try await handle.uploadAttachment(srcLocalPath: url.path, fileName: name, permanent: permanent, uploadId: uploadId)
    }

    /// Stops an upload on its way; the host keeps nothing of it.
    public func cancelUpload(uploadId: String) {
        handle?.cancelUpload(uploadId: uploadId)
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
        if feedStreaming {
            let turns = fileFeed.turns()
            thinkingTurns = transcriptMatchesScreen(file: turns, screen: screen) ? turns : []
            return thinkingTurns
        }
        if !force, let last = lastThinkingRead, Date().timeIntervalSince(last) < 15 { return thinkingTurns }
        lastThinkingRead = Date()
        didReadFile = true
        if let found = try? await handle.detectSessionTranscriptPathIn(sessionName: session.sessionName, kind: multiplexer),
           let turns = await syncFileTurns(handle, found) {
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
            if let found = try? await handle.detectSessionTranscriptPathIn(sessionName: session.sessionName, kind: multiplexer),
               found != activeTranscriptPath {
                activeTranscriptPath = found
                transcriptTurns = []
            }
        }
        guard let path = activeTranscriptPath, let turns = await syncFileTurns(handle, path) else { return false }
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

    /// Opens a file named in the feed. An absolute path is read as it is; `~/` is resolved to the host's home folder; a relative one is tried
    /// under the repository root and then under the pane's folder. The sheet opens at once and fills
    /// when the file arrives (or shows what went wrong).
    public func openFileReference(path: String, line: Int?) {
        guard let handle = handle, state == .connected else {
            fileBrowserError = "Session is not connected"
            return
        }
        filePreview = nil
        fileBrowserError = nil
        fileReference = FileReferenceTarget(path: path, line: line)
        Task {
            var candidates: [String] = []
            if path.hasPrefix("/") {
                candidates = [path]
            } else if path.hasPrefix("~/") {
                // The host's own answer for `~` (a listing of the home folder names it), not a guess.
                if let home = try? await handle.listRemoteDirectory(path: "~", includeHidden: false, helperSha256: await helperPin()).currentPath {
                    candidates = [home.trimmingCharacters(in: CharacterSet(charactersIn: "/")).isEmpty ? path : home + "/" + path.dropFirst(2)]
                }
            } else {
                let rel = path.hasPrefix("./") ? String(path.dropFirst(2)) : path
                if let root = gitRepoRoot { candidates.append(root + "/" + rel) }
                if let cwd = try? await handle.listRemoteDirectory(path: nil, includeHidden: false, helperSha256: await helperPin()).currentPath,
                   !candidates.contains(cwd + "/" + rel) {
                    candidates.append(cwd + "/" + rel)
                }
            }
            for candidate in candidates {
                if let preview = try? await handle.readRemoteFilePreview(path: candidate, maxBytes: 131_072, helperSha256: await helperPin()) {
                    self.filePreview = preview
                    return
                }
            }
            self.fileBrowserError = "Could not open \(path): the file is not under the project or the current folder"
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


/// Carries the stream's events to the main actor.
private final class FeedStreamBridge: FeedListener {
    private weak var vm: TerminalSessionViewModel?
    init(_ vm: TerminalSessionViewModel) { self.vm = vm }
    func onChange(change: FeedChange, cursor: FeedCursor) {
        Task { @MainActor in vm?.streamChanged(cursor) }
    }
    func onLive(text: String) {
        Task { @MainActor in vm?.scheduleStreamReload() }
    }
    func onClosed() {
        Task { @MainActor in vm?.streamClosed() }
    }
}


/// What the feed asked to open: the path as written and the line to show.
public struct FileReferenceTarget: Identifiable, Equatable {
    public let path: String
    public let line: Int?
    public var id: String { path + ":" + (line.map(String.init) ?? "") }
}
