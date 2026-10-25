import SwiftUI
import PhotosUI
import ImageIO
import UniformTypeIdentifiers
import SshCoreBridge

public struct StructuredTranscriptView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var promptText: String = ""
    /// Position in the list of my earlier messages while ↑/↓ browse it (0 = the newest); `nil`: not browsing.
    @State private var historyIndex: Int?
    /// What the field held before browsing started; ↓ past the newest message brings it back.
    @State private var historyStash = ""
    /// The text taken from the agent's box into the field by the up button; it leaves the field
    /// again when the agent's box empties without the text having been edited.
    @State private var adoptedText: String?
    @State private var blinkRed = false
    /// The offered text the reader swiped away; the offer returns only for a different text.
    @State private var dismissedOffer: String?
    @State private var offerDrag: CGFloat = 0
    @State private var blinkTask: Task<Void, Never>?
    @AppStorage(FeedPresentationStyle.storageKey) private var feedStyleRaw: String = FeedPresentationStyle.fallback.rawValue
    /// Tap on the branch / worktree names: shortened names shown whole (and back).
    /// Long press on the git names opens the matching git screen.
    private var look: FeedPresentationStyle { FeedPresentationStyle.resolved(feedStyleRaw) }
    @State private var atBottom = true
    /// Pinch in the feed changes this (0.75-1.8); kept across launches.
    @AppStorage("feedTextScale") private var feedTextScale: Double = 1.0
    @State private var unseenNew = false
    @State private var scrollToken = 0
    @State private var findToken = 0
    @State private var highlightVersion = 0
    @Environment(\.compactChat) private var compactChat
    @AppStorage(SyntaxSettings.themeKey) private var syntaxTheme = SyntaxTheme.auto.rawValue
    @AppStorage(SyntaxSettings.enabledKey) private var syntaxOn = true
    @Environment(\.colorScheme) private var colorScheme
    @State private var editorExpanded = false
    @FocusState private var inputFocused: Bool
    @State private var expandedThinkingTurns: Set<String> = []
    @State private var expandedToolCalls: Set<String> = []
    // Attachments: files go to the host's inbox, the path lands in the message.
    @State private var showPhotoPicker = false
    @State private var showFileImporter = false
    @State private var pickedPhotos: [PhotosPickerItem] = []
    @State private var uploads: [FeedUpload] = []
    @State private var artifacts: [ExtractedArtifact] = []
    @State private var showArtifacts = false
    /// Picked but not sent yet: the user is asked first whether the host keeps them.
    @State private var pendingPhotos: [PhotosPickerItem] = []
    @State private var pendingFiles: [URL] = []
    @State private var askKeep = false
    @State private var pendingClipboard = false
    @State private var showCamera = false
    @State private var uploadTasks: [UUID: Task<Void, Never>] = [:]
    @State private var pendingCamera: [Data] = []
    @State private var syncTask: Task<Void, Never>?
    @State private var adopting = false

    /// The usage numbers shown in the feed's line (`nil`: usage strip switched off).
    var usage: SessionUsageMonitor?
    var onRefreshUsage: (() -> Void)?


    public init(viewModel: TerminalSessionViewModel) {
        self.viewModel = viewModel
    }

    init(viewModel: TerminalSessionViewModel, usage: SessionUsageMonitor?, onRefreshUsage: (() -> Void)?) {
        self.viewModel = viewModel
        self.usage = usage
        self.onRefreshUsage = onRefreshUsage
    }

    /// The one line above the feed: the same line as above the terminal.
    private var feedStrip: some View {
        SessionStatusLine(
            viewModel: viewModel,
            usage: usage ?? SessionUsageMonitor.none,
            onRefresh: onRefreshUsage,
            switchButton: .init(icon: "terminal.fill", title: "Terminal", hint: "Switch this tab back to the raw terminal")
        )
    }

    public var body: some View {
        decorated(layout)
    }

    private var layout: some View {
        VStack(spacing: 0) {
            feedStrip
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
                        .foregroundColor(.white)
                        .glassCard(cornerRadius: 14, tint: (Color.appAccent), interactive: true)
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
                            .foregroundColor(.white)
                            .glassCard(cornerRadius: 14, tint: (Color.appAccent), interactive: true)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.appBackground)
            } else {
                FeedTextView(
                    turns: viewModel.turnsWithEchoes,
                    expandedThinking: expandedThinkingTurns,
                    expandedTools: expandedToolCalls,
                    onToggle: { toggle in
                        switch toggle {
                        case .thinking(let id):
                            if expandedThinkingTurns.contains(id) { expandedThinkingTurns.remove(id) } else { expandedThinkingTurns.insert(id) }
                        case .cancelQueued(let id):
                            viewModel.cancelEcho(id: id)
                        case .resend(let id, let interrupt):
                            viewModel.resendEcho(id: id, interrupt: interrupt)
                        case .toolBack(let id, let all):
                            if all || !expandedToolCalls.contains(id + "+") {
                                expandedToolCalls.remove(id)
                            }
                            expandedToolCalls.remove(id + "+")
                        case .tool(let id):
                            // closed -> open -> fully open -> closed ("id+" marks the last stage)
                            if id.hasPrefix("svc:") || id.contains("#") {
                                if expandedToolCalls.contains(id) { expandedToolCalls.remove(id) } else { expandedToolCalls.insert(id) }
                            } else if !expandedToolCalls.contains(id) {
                                expandedToolCalls.insert(id)
                            } else if !expandedToolCalls.contains(id + "+") {
                                expandedToolCalls.insert(id + "+")
                            } else {
                                expandedToolCalls.remove(id)
                                expandedToolCalls.remove(id + "+")
                            }
                        }
                    },
                    atBottom: $atBottom,
                    scrollToken: scrollToken,
                    findToken: findToken,
                    dark: colorScheme == .dark,
                    highlightVersion: highlightVersion,
                    textScale: CGFloat(feedTextScale),
                    onScale: { feedTextScale = Double($0) },
                    presentation: look,
                    onOpenFile: { viewModel.openFileReference(path: $0, line: $1) }
                )
                .background(Color.appBackground)
                .task(id: "\(viewModel.transcriptTurns.count)|\(viewModel.transcriptTurns.last?.content?.count ?? 0)|\(colorScheme == .dark)|\(syntaxTheme)|\(syntaxOn)") {
                    if await FeedHighlightStore.prefetch(viewModel.transcriptTurns, dark: colorScheme == .dark) {
                        highlightVersion += 1
                    }
                }
                .task(id: "\(viewModel.transcriptTurns.count)|\(viewModel.transcriptTurns.last?.content?.count ?? 0)") {
                    artifacts = Self.collectArtifacts(viewModel.transcriptTurns)
                }
                .overlay(alignment: .bottomLeading) {
                    if !artifacts.isEmpty {
                        Button { showArtifacts = true } label: {
                            Label("Diagrams \(artifacts.count)", systemImage: "point.3.connected.trianglepath.dotted")
                                .font(.system(size: 12, weight: .semibold))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .glassChip()
                        }
                        .padding(10)
                    }
                }
                .sheet(isPresented: $showArtifacts) { ArtifactListSheet(artifacts: artifacts) }
                .onChange(of: viewModel.transcriptTurns) { _, _ in
                    viewModel.pruneEchoes()
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
                                .glassChip()
                                .overlay(alignment: .topTrailing) {
                                    if unseenNew { Circle().fill(Color.appAccent).frame(width: 8, height: 8) }
                                }
                        }
                        .padding(10)
                    }
                }
            }

            if let prompt = viewModel.screenPrompt {
                FeedPromptCard(prompt: prompt,
                               pick: { viewModel.answerPrompt(index: $0, toggle: prompt.multi) },
                               pickText: { viewModel.answerPrompt(index: $0, text: $1) },
                               finish: { viewModel.finishPrompt(confirm: $0) },
                               tab: { viewModel.moveTab(back: $0) })
            }

            if let aside = viewModel.screenAside {
                FeedAsideCard(aside: aside, close: { viewModel.closeOverlay() },
                              more: { viewModel.sendDialogKeys([[0x1b, 0x5b, 0x36, 0x7e]]) })
            }

            // The agent works: a band of light runs along the border between the feed and the input bar.
            Divider()
                .overlay {
                    if let status = viewModel.screenStatus {
                        WorkingLightBar().accessibilityElement().accessibilityLabel(status)
                    }
                }

            // Bottom prompt: a multi-line editor with a few control keys above it.
            VStack(spacing: 6) {
                // The system keyboard has none of these; a row that scrolls sideways, in groups.
                if !compactChat {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(FeedKeys.all) { key in
                            feedKey(key.title) {
                                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                if (key.title == "↑" || key.title == "↓"), browseHistory(up: key.title == "↑") { return }
                                viewModel.sendInput(Data(key.bytes))
                            }
                            if key.endsGroup { Divider().frame(height: 18) }
                        }
                    }
                }
                }
                if let queued = offeredDraft { queuedStrip(queued) }
                if !uploads.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(uploads) { u in uploadChip(u) }
                        }
                    }
                }
                GlassEffectContainer(spacing: 10) {
                HStack(alignment: .bottom, spacing: 8) {
                    Menu {
                        Button { showPhotoPicker = true } label: { Label("Photos & videos", systemImage: "photo.on.rectangle") }
                        Button { showFileImporter = true } label: { Label("Files", systemImage: "folder") }
                        if UIImagePickerController.isSourceTypeAvailable(.camera) {
                            Button { showCamera = true } label: { Label("Take photo", systemImage: "camera") }
                        }
                        if UIPasteboard.general.hasImages {
                            Button { pendingClipboard = true; askKeep = true } label: { Label("Paste image", systemImage: "doc.on.clipboard") }
                        }
                        if hasAgentCommands {
                            Divider()
                            Menu { AgentCommandItems(agy: viewModel.isAgyAgent) { viewModel.runAgentCommand($0) } } label: {
                                Label("Commands", systemImage: "slash.circle")
                            }
                        }
                    } label: {
                        Image(systemName: "paperclip.circle.fill")
                            .font(.system(size: 22))
                            .foregroundColor(.appAccent)
                    }
                    .buttonStyle(.glass)
                    .buttonBorderShape(.circle)
                    .disabled(viewModel.sessionHandle == nil || viewModel.state != .connected)
                    TextField("Message…", text: $promptText, axis: .vertical)
                        .lineLimit(editorExpanded ? 8...16 : 1...6)
                        .focused($inputFocused)
                        .font(.system(size: 16))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 9)
                    .glassCard(cornerRadius: 19)
                    .overlay(alignment: .topTrailing) {
                        Button { withAnimation { editorExpanded.toggle() } } label: {
                            Image(systemName: editorExpanded
                                ? "arrow.down.right.and.arrow.up.left"
                                : "arrow.up.left.and.arrow.down.right")
                                .font(.system(size: 11))
                                .foregroundColor(.secondary)
                                .padding(8)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(editorExpanded ? "Shrink the message box" : "Enlarge the message box")
                    }

                    if viewModel.screenStatus != nil, viewModel.screenPrompt == nil {
                        // The agent is working: two ways to get a message to it right now.
                        Menu {
                            Button(action: sendImmediately) {
                                Label("Interrupt and send", systemImage: "stop.circle")
                            }
                            Button(action: sendPrompt) {
                                Label("Add to the current task", systemImage: "text.badge.plus")
                            }
                        } label: {
                            Image(systemName: "bolt.fill").font(.system(size: 16, weight: .semibold))
                        }
                        .buttonStyle(.glass)
                        .buttonBorderShape(.circle)
                        .tint(canSend ? .orange : .gray)
                        .disabled(!canSend)
                        .accessibilityLabel("Send now")
                    }
                    Button(action: sendPrompt) {
                        GlassRoundIcon(symbol: "arrow.up", tint: canSend ? .appAccent : nil, symbolColor: .secondary)
                    }
                    .disabled(!canSend)
                }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color.appBackground)
        }
    }

    /// Closed ```mermaid / markdown / html fences in the agent's replies, newest last, each once.
    static func collectArtifacts(_ turns: [TranscriptTurn]) -> [ExtractedArtifact] {
        var seen = Set<String>()
        var out: [ExtractedArtifact] = []
        for turn in turns where turn.turnType == .assistant {
            guard let text = turn.content, text.contains("```") || text.contains("~~~") else { continue }
            for a in extractArtifacts(text: text) where seen.insert(a.id).inserted { out.append(a) }
        }
        return out
    }

    private func decorated<V: View>(_ content: V) -> some View {
        content
        .sheet(isPresented: Binding(get: { viewModel.screenDialog != nil },
                                    set: { if !$0 { viewModel.closeOverlay() } })) {
            if let dialog = viewModel.screenDialog {
                CommandScreenView(dialog: dialog, send: { viewModel.sendDialogKeys($0) }, close: { viewModel.closeOverlay() })
                    .presentationDetents([.medium, .large])
            }
        }
        .sheet(item: $viewModel.fileReference, onDismiss: { viewModel.filePreview = nil }) { target in
            FilePreviewView(viewModel: viewModel, focusLine: target.line)
        }
        .photosPicker(isPresented: $showPhotoPicker, selection: $pickedPhotos, maxSelectionCount: 6,
                      matching: .any(of: [.images, .videos]))
        .onChange(of: pickedPhotos) { _, items in
            guard !items.isEmpty else { return }
            pickedPhotos = []
            pendingPhotos += items
            askKeep = true
        }
        .fullScreenCover(isPresented: $showCamera) {
            CameraPicker { data in
                if let data { pendingCamera.append(data); askKeep = true }
            }
            .ignoresSafeArea()
        }
        .fileImporter(isPresented: $showFileImporter, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            pendingFiles += urls
            askKeep = true
        }
        .confirmationDialog("Keep on the host?", isPresented: $askKeep, titleVisibility: .visible) {
            Button("Keep permanently") { sendPending(permanent: true) }
            Button("Temporary") { sendPending(permanent: false) }
            Button("Cancel", role: .cancel) { pendingPhotos = []; pendingFiles = []; pendingClipboard = false; pendingCamera = [] }
        } message: {
            Text("Permanent: ~/.ssh-app/inbox, stays until you delete it. Temporary: the host's temp folder, which the system empties.")
        }
        .onChange(of: viewModel.feedFindRequests) { _, _ in findToken += 1 }
        .onAppear {
            if viewModel.transcriptTurns.isEmpty {
                viewModel.loadTranscript()
            }
            draftChanged(viewModel.screenDraft)
        }
        .onChange(of: promptText) { _, _ in
            if adopting { adopting = false } else { scheduleDraftSync() }
        }
        .onChange(of: viewModel.screenDraft) { _, new in draftChanged(new) }
        // Live tail: only while this view is on screen (the task ends with it). The host pushes
        // changes over one open channel; this loop only keeps that channel alive and, when it
        // cannot run (Zellij, no agent file, a host without a shell), falls back to polling:
        // 3 s while the transcript moves; 5 s, then 10 s when it stands still.
        .task {
            var quiet = 0
            while !Task.isCancelled {
                await viewModel.ensureFeedStream()
                if viewModel.feedStreaming {
                    try? await Task.sleep(for: .seconds(20))   // a safety look; events do the work
                    _ = await viewModel.reloadTranscriptQuietly()
                    quiet = 0
                } else {
                    try? await Task.sleep(for: .seconds(quiet >= 6 ? 10 : (quiet >= 2 ? 5 : 3)))
                    quiet = await viewModel.reloadTranscriptQuietly() ? 0 : quiet + 1
                }
            }
        }
        .onDisappear { viewModel.stopFeedStream() }
    }

    private func uploadChip(_ u: FeedUpload) -> some View {
        HStack(spacing: 5) {
            if let thumb = u.thumbnail {
                Image(uiImage: thumb)
                    .resizable().scaledToFill()
                    .frame(width: 28, height: 28)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    .overlay {
                        switch u.state {
                        case .sending: ProgressView().controlSize(.mini).padding(3).glassEffect(.regular, in: Circle())
                        case .failed: Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.orange)
                        }
                    }
            } else {
                switch u.state {
                case .sending: ProgressView().controlSize(.mini)
                case .failed: Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.orange)
                }
            }
            Text(u.failure.map { "\(u.name): \($0)" } ?? u.name)
                .font(.system(size: 11, design: .monospaced))
                .lineLimit(1)
            Button { cancelUpload(u.id) } label: { Image(systemName: "xmark.circle.fill") }
                .foregroundColor(.secondary)
                .accessibilityLabel(u.state == .failed ? "Dismiss" : "Cancel upload")
        }
        .padding(.horizontal, 8).padding(.vertical, 4)
        .glassChip()
    }

    private func sendPending(permanent: Bool) {
        let photos = pendingPhotos, files = pendingFiles, clip = pendingClipboard
        pendingPhotos = []
        pendingFiles = []
        pendingClipboard = false
        let shots = pendingCamera
        pendingCamera = []
        for data in shots { startUpload { await attachCameraShot($0, data, permanent: permanent) } }
        if clip { startUpload { await attachClipboardImage($0, permanent: permanent) } }
        for item in photos { startUpload { await attachPhoto($0, item, permanent: permanent) } }
        for url in files { startUpload { await attachFile($0, url, permanent: permanent) } }
    }

    /// Runs one upload as a task that the chip's cross can cancel.
    private func startUpload(_ work: @escaping (UUID) async -> Void) {
        let id = UUID()
        uploadTasks[id] = Task {
            await work(id)
            uploadTasks[id] = nil
        }
    }

    /// Stops an upload on its way (the host throws away what had arrived) or clears a failed one.
    private func cancelUpload(_ id: UUID) {
        uploadTasks[id]?.cancel()
        uploadTasks[id] = nil
        viewModel.cancelUpload(uploadId: id.uuidString)
        uploads.removeAll { $0.id == id }
    }

    private func attachPhoto(_ id: UUID, _ item: PhotosPickerItem, permanent: Bool) async {
        let label = item.supportedContentTypes.first?.preferredFilenameExtension.map { "photo.\($0)" } ?? "photo"
        uploads.append(FeedUpload(id: id, name: label))
        guard let picked = try? await item.loadTransferable(type: PickedFile.self) else {
            fail(id, "could not read")
            return
        }
        defer { try? FileManager.default.removeItem(at: picked.url) }
        if ImagePreflight.isImage(picked.url) {
            // A picture never leaves the phone with its metadata: if it cannot be re-encoded, nothing is sent.
            guard let clean = try? ImagePreflight.processToTemporaryFile(picked.url, baseName: picked.name) else {
                fail(id, "not a readable image")
                return
            }
            defer { try? FileManager.default.removeItem(at: clean.deletingLastPathComponent()) }
            await send(id, url: clean, name: clean.lastPathComponent, permanent: permanent, isImage: true)
            return
        }
        await send(id, url: picked.url, name: picked.name, permanent: permanent)
    }

    private func attachFile(_ id: UUID, _ url: URL, permanent: Bool) async {
        uploads.append(FeedUpload(id: id, name: url.lastPathComponent))
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        if ImagePreflight.isImage(url), let clean = try? ImagePreflight.processToTemporaryFile(url, baseName: url.lastPathComponent) {
            defer { try? FileManager.default.removeItem(at: clean.deletingLastPathComponent()) }
            await send(id, url: clean, name: clean.lastPathComponent, permanent: permanent, isImage: true)
            return
        }
        await send(id, url: url, name: url.lastPathComponent, permanent: permanent)
    }

    /// A photo just taken, processed like a picked one.
    private func attachCameraShot(_ id: UUID, _ data: Data, permanent: Bool) async {
        uploads.append(FeedUpload(id: id, name: "camera"))
        guard let r = try? ImagePreflight.process(data) else {
            fail(id, "not a readable image")
            return
        }
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let file = dir.appendingPathComponent("camera.\(r.ext)")
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try r.data.write(to: file, options: .atomic)
        } catch {
            fail(id, error.localizedDescription)
            return
        }
        defer { try? FileManager.default.removeItem(at: dir) }
        await send(id, url: file, name: file.lastPathComponent, permanent: permanent, isImage: true)
    }

    /// A picture from the system clipboard, processed like a picked one.
    private func attachClipboardImage(_ id: UUID, permanent: Bool) async {
        uploads.append(FeedUpload(id: id, name: "clipboard"))
        guard let data = UIPasteboard.general.data(forPasteboardType: UTType.image.identifier)
                ?? UIPasteboard.general.image?.pngData(),
              let r = try? ImagePreflight.process(data) else {
            fail(id, "no image on the clipboard")
            return
        }
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let file = dir.appendingPathComponent("clipboard.\(r.ext)")
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try r.data.write(to: file, options: .atomic)
        } catch {
            fail(id, error.localizedDescription)
            return
        }
        defer { try? FileManager.default.removeItem(at: dir) }
        await send(id, url: file, name: file.lastPathComponent, permanent: permanent, isImage: true)
    }

    private func send(_ id: UUID, url: URL, name: String, permanent: Bool, isImage: Bool = false) async {
        if isImage, let i = uploads.firstIndex(where: { $0.id == id }) {
            uploads[i].thumbnail = ImagePreflight.thumbnail(of: url)
        }
        do {
            let path = try await viewModel.uploadAttachment(url, name: name, permanent: permanent, uploadId: id.uuidString)
            // Cancelled in the last moment: the file is on the host, but the user said no.
            if Task.isCancelled { return }
            uploads.removeAll { $0.id == id }
            // the path goes into the message where the cursor was left: on its own, space-separated
            let sep = promptText.isEmpty || promptText.hasSuffix(" ") || promptText.hasSuffix("\n") ? "" : " "
            // Claude Code reads a picture it is given as @<path>; other agents get the bare path.
            let ref = isImage ? viewModel.traits.imageReferencePrefix + path : path
            promptText += sep + ref + " "
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        } catch {
            if Task.isCancelled || error is CancellationError || (error as? SshCoreError) == .Cancelled { return }
            fail(id, error.localizedDescription)
        }
    }

    private func fail(_ id: UUID, _ message: String) {
        if let i = uploads.firstIndex(where: { $0.id == id }) {
            uploads[i].state = .failed
            uploads[i].failure = String(message.prefix(60))
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
                .glassChip(interactive: true)
        }
        .buttonStyle(.plain)
    }

    /// Slash commands of Claude Code or Antigravity; a plain tab shows the feed only once an agent runs in it.
    private var hasAgentCommands: Bool {
        switch viewModel.session.agentType {
        case .localLlm: return false
        case .claudeCode, .antigravity, .customShell, .dockerContainer: return true
        }
    }

    private func sendPrompt() { send(.normal) }

    private func sendImmediately() {
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        send(.interrupt)
    }

    /// Clears the field at once and hands the text to the view model, which fills the agent's box,
    /// presses Enter and checks the box. A message that did not go out comes back into the field.
    private func send(_ mode: SubmitMode) {
        guard canSend else { return }
        let text = promptText
        promptText = ""
        syncTask?.cancel()
        Task {
            if await viewModel.submit(text, mode: mode) == .notSent, promptText.isEmpty {
                promptText = text
            }
        }
    }

    /// Mirrors the message field into the agent's input box a moment after you stop typing.
    private func scheduleDraftSync() {
        syncTask?.cancel()
        let text = promptText
        syncTask = Task {
            try? await Task.sleep(for: .milliseconds(450))
            guard !Task.isCancelled, !viewModel.sendingNow else { return }
            _ = await viewModel.syncDraft(text)
        }
    }

    /// What the agent's box holds (a message typed at the laptop, or queued while it works) when the
    /// field is empty: offered above the field, not poured into it.
    private var offeredDraft: String? {
        guard promptText.isEmpty else { return nil }
        let offer: String?
        if let queued = viewModel.queuedNote, viewModel.screenStatus != nil { offer = queued } else { offer = viewModel.offerableScreenDraft }
        return offer == dismissedOffer ? nil : offer
    }

    /// The up arrow blinks red for 4 seconds when something new is offered, then stops; a tap on it
    /// sends the message straight to the agent, a tap on the text moves it into the field to edit.
    private func queuedStrip(_ text: String) -> some View {
        HStack(spacing: 8) {
            Button { submitScreenDraft() } label: {
                GlassRoundIcon(symbol: "arrow.up", tint: blinkRed ? .red : .appAccent, size: 32)
            }
            .accessibilityLabel("Send the message waiting in the agent")
            VStack(alignment: .leading, spacing: 1) {
                Text("Waiting in the agent").font(.caption2.bold()).foregroundStyle(.secondary)
                Text(text).font(.system(size: 12)).lineLimit(2)
            }
            .contentShape(Rectangle())
            .onTapGesture { adoptScreenDraft() }
            .accessibilityHint("Moves it into the message field to edit")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .glassCard(cornerRadius: 16)
        // Swipe sideways to put the offer away; it comes back only for another text.
        .offset(x: offerDrag)
        .opacity(1 - min(0.7, abs(offerDrag) / 200))
        .gesture(
            DragGesture(minimumDistance: 16)
                .onChanged { v in
                    if abs(v.translation.width) > abs(v.translation.height) { offerDrag = v.translation.width }
                }
                .onEnded { v in
                    if abs(v.translation.width) > 80 {
                        dismissedOffer = text
                        viewModel.queuedNote = nil
                        blinkTask?.cancel()
                        blinkRed = false
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    }
                    withAnimation(.easeOut(duration: 0.2)) { offerDrag = 0 }
                }
        )
        .accessibilityAction(named: "Dismiss") { dismissedOffer = text; viewModel.queuedNote = nil }
    }

    private func startBlink() {
        blinkTask?.cancel()
        blinkTask = Task { @MainActor in
            let end = Date().addingTimeInterval(4)
            while Date() < end, !Task.isCancelled {
                blinkRed.toggle()
                try? await Task.sleep(for: .milliseconds(400))
            }
            blinkRed = false
        }
    }

    /// The agent's box changed: new text starts the blink; an empty box takes back what the up
    /// button had moved into the field, unless you changed it.
    private func draftChanged(_ draft: String?) {
        if let d = draft, !d.isEmpty {
            if promptText.isEmpty { startBlink() }
            return
        }
        blinkTask?.cancel()
        blinkRed = false
        if let a = adoptedText, promptText == a {
            adopting = true
            promptText = ""
            viewModel.draftWasSent()
        }
        adoptedText = nil
    }

    /// My messages of this conversation, newest first, without repeats of the one before.
    private var sentHistory: [String] {
        var out: [String] = []
        for t in viewModel.turnsWithEchoes.reversed() where t.turnType == .user {
            let text = (t.content ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty, out.last != text { out.append(text) }
        }
        return out
    }

    /// ↑ / ↓ as in the CLI: with an empty field (or while browsing) they walk through my earlier
    /// messages, putting each into the field. Returns `false` when the key should go to the agent as
    /// an ordinary arrow (text is being written, or there is nothing to browse).
    private func browseHistory(up: Bool) -> Bool {
        let list = sentHistory
        func put(_ text: String) {
            if text != promptText { adopting = true; promptText = text }
        }
        if promptText.isEmpty { historyIndex = nil }
        guard let i = historyIndex else {
            guard up, promptText.isEmpty, !list.isEmpty else { return false }
            historyStash = ""
            historyIndex = 0
            put(list[0])
            return true
        }
        // The text was edited by hand: browsing is over, the arrows are ordinary again.
        guard i < list.count, promptText == list[i] else { historyIndex = nil; return false }
        if up {
            if i + 1 < list.count { historyIndex = i + 1; put(list[i + 1]) }
        } else if i == 0 {
            historyIndex = nil
            put(historyStash)
        } else {
            historyIndex = i - 1
            put(list[i - 1])
        }
        return true
    }

    /// Sends what waits in the agent right now. An idle agent has the text in its box and only Enter
    /// is missing. A working one queues it on Enter, and its own "send now" (Ctrl+X, Ctrl+S) then
    /// puts the queued message first.
    private func submitScreenDraft() {
        guard viewModel.state == .connected, offeredDraft != nil else { return }
        blinkTask?.cancel()
        blinkRed = false
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        if viewModel.queuedNote != nil, viewModel.screenStatus != nil {
            // Already queued: only the agent's own "send now" is missing.
            viewModel.queuedNote = nil
            viewModel.sendQueuedNow()
            Task {
                try? await Task.sleep(for: .seconds(1))
                _ = await viewModel.reloadTranscriptQuietly()
            }
            return
        }
        let working = viewModel.screenStatus != nil
        viewModel.draftIsBeingSent()
        Task {
            if working {
                viewModel.sendInput(Data([0x0d]))
                try? await Task.sleep(for: .milliseconds(450))
                viewModel.sendQueuedNow()
            } else {
                await viewModel.submitBox(acceptSuggestion: true)
            }
            viewModel.draftWasSent()
            try? await Task.sleep(for: .seconds(1))
            _ = await viewModel.reloadTranscriptQuietly()
        }
    }

    /// Moves the text of the agent's box into the message field (as long as nothing else is written).
    private func adoptScreenDraft() {
        guard let adopted = viewModel.adoptableDraft(current: promptText) else { return }
        blinkTask?.cancel()
        blinkRed = false
        adopting = true
        promptText = adopted
        adoptedText = adopted
    }

}

/// Keeps the agent's input box equal to the text in the feed's message field, so a half-written
/// prompt lives on the host (in the tmux pane) and survives a dropped connection or a reinstall.
/// Only the difference is typed: the common start stays, the rest is deleted with Backspace and
/// the new end typed. The cursor is taken to be at the end of the box.
enum DraftSync {
    /// Most Backspaces one edit may send (a longer draft is not worth a burst of keys).
    static let maxDeletes = 4000

    /// Bytes that turn `old` (what the box holds) into `new`. `uncertain`: `old` was read back from
    /// the screen, where wrapped lines may have lost a space, so delete generously and retype all.
    static func bytes(from old: String, to new: String, uncertain: Bool = false) -> [UInt8]? {
        let clean = sanitized(new)
        if clean == old, !uncertain { return [] }
        let a = Array(old), b = Array(clean)
        var common = 0
        if !uncertain {
            while common < a.count, common < b.count, a[common] == b[common] { common += 1 }
        }
        let deletes = uncertain ? a.count * 2 + 20 : a.count - common
        if deletes > maxDeletes { return nil }
        var out = [UInt8](repeating: 0x7f, count: deletes)
        let tail = String(b[common...])
        if tail.contains("\n") {
            out += Array("\u{1b}[200~".utf8) + Array(tail.utf8) + Array("\u{1b}[201~".utf8)
        } else {
            out += Array(tail.utf8)
        }
        return out
    }

    /// What the box will show for `text`: no control characters (they would act as keys), no paste
    /// markers (they could end the paste early), one kind of line break.
    static func sanitized(_ text: String) -> String {
        let t = text
            .replacingOccurrences(of: "\u{1b}[201~", with: "")
            .replacingOccurrences(of: "\u{1b}[200~", with: "")
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        return String(t.unicodeScalars.filter { $0 == "\n" || ($0.value >= 0x20 && $0.value != 0x7f) })
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


/// The agent's question as a card with buttons, instead of arrow keys in a terminal.
struct FeedPromptCard: View {
    let prompt: ScreenPrompt
    let pick: (Int) -> Void
    /// Moves to the free-text option and types the text into it.
    let pickText: (Int, String) -> Void
    let finish: (Bool) -> Void
    let tab: (Bool) -> Void

    /// The option that waits for your own words ("Type something.", "Other"): a text field, not a
    /// button, otherwise the agent gets an empty answer.
    @State private var typingIn: Int?
    @State private var typed = ""
    @FocusState private var typedFocused: Bool

    private static func isFreeText(_ label: String) -> Bool {
        let l = label.lowercased().trimmingCharacters(in: .whitespaces)
        return l.hasPrefix("type something") || l == "other" || l == "other."
    }

    /// "☒ A ☐ B ✔ Submit" → ["☒ A", "☐ B", "✔ Submit"]
    private static func tabs(_ s: String) -> [String] {
        var out: [String] = []
        for ch in s {
            if "☒☐✔☑".contains(ch) || out.isEmpty { out.append(String(ch)) } else { out[out.count - 1].append(ch) }
        }
        return out.map { $0.trimmingCharacters(in: .whitespaces) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title = prompt.title {
                if title.contains("Submit") {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(Array(Self.tabs(title).enumerated()), id: \.offset) { _, t in
                                let done = t.hasPrefix("☒") || t.hasPrefix("✔")
                                Text(t)
                                    .font(.system(size: 11, weight: .semibold))
                                    .padding(.horizontal, 8).padding(.vertical, 3)
                                    .background((done ? Color.appAccent : Color.secondary).opacity(0.18), in: Capsule())
                            }
                        }
                    }
                } else {
                    Text(title.uppercased())
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            if !prompt.detail.isEmpty {
                Text(prompt.detail)
                    .font(.system(size: 12.5, design: .monospaced))
                    .lineLimit(14)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
            }
            Text(prompt.question)
                .font(.system(size: 14, weight: .semibold))
            ForEach(Array(prompt.options.enumerated()), id: \.offset) { i, o in
                Button {
                    if Self.isFreeText(o.label) {
                        typingIn = typingIn == i ? nil : i
                        typedFocused = typingIn != nil
                    } else {
                        pick(i)
                    }
                } label: {
                    HStack(alignment: .top, spacing: 8) {
                        if prompt.multi {
                            Image(systemName: o.checked ? "checkmark.square.fill" : "square")
                        } else {
                            Text("\(o.number)").font(.system(size: 12, weight: .bold, design: .monospaced))
                                .frame(width: 18)
                        }
                        VStack(alignment: .leading, spacing: 2) {
                            Text(o.label).font(.system(size: 13, weight: .medium)).multilineTextAlignment(.leading)
                            if !o.description.isEmpty {
                                Text(o.description).font(.system(size: 11.5)).foregroundStyle(.secondary)
                                    .multilineTextAlignment(.leading)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 10).padding(.vertical, 7)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(i == Int(prompt.selected) ? Color.appAccent.opacity(0.16) : Color.secondary.opacity(0.1),
                                in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8)
                        .stroke(i == Int(prompt.selected) ? Color.appAccent.opacity(0.6) : Color.clear, lineWidth: 1))
                }
                .buttonStyle(.plain)
                if typingIn == i {
                    HStack(spacing: 8) {
                        TextField("Your answer", text: $typed, axis: .vertical)
                            .textFieldStyle(.roundedBorder)
                            .lineLimit(1...5)
                            .focused($typedFocused)
                        Button {
                            let text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !text.isEmpty else { return }
                            pickText(i, text)
                            typed = ""
                            typingIn = nil
                        } label: { GlassRoundIcon(symbol: "arrow.up", tint: .appAccent, size: 30) }
                            .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            HStack {
                if prompt.title?.contains("Submit") == true {
                    Button { tab(true) } label: { Label("Back", systemImage: "chevron.left") }.buttonStyle(.glass)
                    Button { tab(false) } label: { Label("Next", systemImage: "chevron.right") }.buttonStyle(.glassProminent)
                }
                // in a multi-question form Enter only toggles; "Done" is the next tab (the last one is Submit)
                if prompt.multi && prompt.title?.contains("Submit") != true {
                    Button("Done") { finish(true) }.buttonStyle(.glassProminent)
                }
                Spacer()
                Button("Esc") { finish(false) }.buttonStyle(.glass)
            }
            .font(.system(size: 13))
        }
        .padding(12)
        .background(Color.appBackground)
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.secondary.opacity(0.35), lineWidth: 1))
        .padding(.horizontal, 10).padding(.vertical, 6)
    }
}


/// Keys for the feed's key row (bytes as a terminal sends them).
struct FeedKey: Identifiable {
    let title: String
    let bytes: [UInt8]
    var endsGroup = false
    var id: String { title }
}

enum FeedKeys {
    private static func esc(_ s: String) -> [UInt8] { [0x1b] + Array(s.utf8) }
    private static func ctrl(_ c: Character) -> [UInt8] { [UInt8(c.asciiValue! & 0x1f)] }

    static let all: [FeedKey] = [
        FeedKey(title: "Esc", bytes: [0x1b]),
        FeedKey(title: "Tab", bytes: [0x09]),
        FeedKey(title: "⇧Tab", bytes: esc("[Z")),
        FeedKey(title: "⏎", bytes: [0x0d], endsGroup: true),
        FeedKey(title: "↑", bytes: esc("[A")),
        FeedKey(title: "↓", bytes: esc("[B")),
        FeedKey(title: "←", bytes: esc("[D")),
        FeedKey(title: "→", bytes: esc("[C")),
        FeedKey(title: "Home", bytes: esc("[H")),
        FeedKey(title: "End", bytes: esc("[F")),
        FeedKey(title: "PgUp", bytes: esc("[5~")),
        FeedKey(title: "PgDn", bytes: esc("[6~"), endsGroup: true),
        FeedKey(title: "^C", bytes: ctrl("c")),
        FeedKey(title: "^D", bytes: ctrl("d")),
        FeedKey(title: "^Z", bytes: ctrl("z")),
        FeedKey(title: "^L", bytes: ctrl("l")),
        FeedKey(title: "^R", bytes: ctrl("r")),
        FeedKey(title: "^A", bytes: ctrl("a")),
        FeedKey(title: "^E", bytes: ctrl("e")),
        FeedKey(title: "^U", bytes: ctrl("u")),
        FeedKey(title: "^W", bytes: ctrl("w")),
        FeedKey(title: "^K", bytes: ctrl("k")),
        FeedKey(title: "^O", bytes: ctrl("o"), endsGroup: true),
        FeedKey(title: "y", bytes: Array("y".utf8)),
        FeedKey(title: "n", bytes: Array("n".utf8)),
        FeedKey(title: "/", bytes: Array("/".utf8)),
        FeedKey(title: "|", bytes: Array("|".utf8)),
        FeedKey(title: "~", bytes: Array("~".utf8)),
        FeedKey(title: "\\", bytes: Array("\\".utf8)),
        FeedKey(title: "-", bytes: Array("-".utf8)),
        FeedKey(title: "_", bytes: Array("_".utf8)),
        FeedKey(title: "`", bytes: Array("`".utf8)),
    ]
}


/// An attachment on its way to the host.
struct FeedUpload: Identifiable {
    enum State { case sending, failed }
    let id: UUID
    var name: String
    var state: State = .sending
    var failure: String?
    var thumbnail: UIImage?
}

/// A photo, video or file taken from the picker, copied to a temporary file the core can read.
struct PickedFile: Transferable {
    let url: URL
    let name: String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .item) { received in
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let dest = dir.appendingPathComponent(received.file.lastPathComponent)
            try FileManager.default.copyItem(at: received.file, to: dest)
            return PickedFile(url: dest, name: received.file.lastPathComponent)
        }
    }
}

/// Pre-flight of a picture before it leaves the phone (feature 38): decoded and written again
/// without any metadata (GPS, device, time), the longest side clamped, HEIC and other formats
/// turned into JPEG; PNG stays PNG (screenshots keep sharp text).
enum ImagePreflight {
    static let maxSide = 2048

    struct Result {
        let data: Data
        let ext: String
        let width: Int
        let height: Int
    }

    enum Failure: Error { case notAnImage, encodeFailed }

    static func process(_ input: Data, maxSide: Int? = nil) throws -> Result {
        let settings = AppConfigStore.current().uploads
        let maxSide = maxSide ?? Int(settings.maxDimension)
        guard let source = CGImageSourceCreateWithData(input as CFData, nil),
              CGImageSourceGetCount(source) > 0 else { throw Failure.notAnImage }
        let isPNG = (CGImageSourceGetType(source) as String?) == UTType.png.identifier
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,   // the orientation is applied, then dropped
            kCGImageSourceThumbnailMaxPixelSize: maxSide,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            throw Failure.notAnImage
        }
        let out = NSMutableData()
        let type = isPNG ? UTType.png : UTType.jpeg
        guard let dest = CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil) else {
            throw Failure.encodeFailed
        }
        // With metadata removal on (the default, and the only thing an import can set) no properties
        // of the source are copied: nothing but pixels is written.
        var props: [CFString: Any] = [:]
        if !settings.stripMetadata,
           let original = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any] {
            props = original
        }
        if !isPNG { props[kCGImageDestinationLossyCompressionQuality] = Double(settings.jpegQualityPercent) / 100 }
        CGImageDestinationAddImage(dest, image, props as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { throw Failure.encodeFailed }
        return Result(data: out as Data, ext: isPNG ? "png" : "jpg", width: image.width, height: image.height)
    }

    /// Writes the processed picture to a temporary file for the upload.
    static func processToTemporaryFile(_ url: URL, baseName: String) throws -> URL {
        let r = try process(try Data(contentsOf: url))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let stem = (baseName as NSString).deletingPathExtension
        let file = dir.appendingPathComponent("\(stem.isEmpty ? "photo" : stem).\(r.ext)")
        try r.data.write(to: file, options: .atomic)
        return file
    }

    /// A small picture for the chip in the composer.
    static func thumbnail(of url: URL, side: Int = 96) -> UIImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: side,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary).map { UIImage(cgImage: $0) }
    }

    static func isImage(_ url: URL) -> Bool {
        guard let t = UTType(filenameExtension: url.pathExtension) else { return false }
        return t.conforms(to: .image)
    }
}

/// The system camera. The photo is handed over as JPEG data and goes through `ImagePreflight` like
/// any other picture; nothing is written to the photo library.
struct CameraPicker: UIViewControllerRepresentable {
    let onDone: (Data?) -> Void
    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let parent: CameraPicker
        init(_ parent: CameraPicker) { self.parent = parent }

        func imagePickerController(_ picker: UIImagePickerController,
                                   didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            let data = (info[.originalImage] as? UIImage)?.jpegData(compressionQuality: 0.9)
            parent.onDone(data)
            parent.dismiss()
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            parent.onDone(nil)
            parent.dismiss()
        }
    }
}


/// A thin solid line with a band of light that runs from left to right and back while the agent
/// works. Keeps still (a plain line) with Reduce Motion.
struct WorkingLightBar: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let period = 1.6

    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { context in
            let t = context.date.timeIntervalSinceReferenceDate / period
            // 0...1...0 with soft ends.
            let phase = 0.5 - 0.5 * cos(t * 2 * .pi)
            GeometryReader { geo in
                let band = max(60, geo.size.width * 0.28)
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.appAccent.opacity(0.28))
                    Capsule()
                        .fill(LinearGradient(colors: [.clear, Color.appAccent, .white.opacity(0.9), Color.appAccent, .clear],
                                             startPoint: .leading, endPoint: .trailing))
                        .frame(width: band)
                        .offset(x: (geo.size.width - band) * (reduceMotion ? 0.5 : phase))
                }
                .clipShape(Capsule())
            }
        }
        .frame(height: 3)
        .padding(.horizontal, 4)
    }
}
