import SwiftUI
import PhotosUI
import UniformTypeIdentifiers
import SshCoreBridge

public struct StructuredTranscriptView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @State private var promptText: String = ""
    @AppStorage(FeedPresentationStyle.storageKey) private var feedStyleRaw: String = FeedPresentationStyle.fallback.rawValue
    /// Tap on the branch / worktree names: shortened names shown whole (and back).
    @State private var gitNamesFull = false
    /// Long press on the git names opens the matching git screen.
    private enum GitSheet: String, Identifiable { case changes, branches, worktrees; var id: String { rawValue } }
    @State private var gitSheet: GitSheet?
    private var look: FeedPresentationStyle { FeedPresentationStyle.resolved(feedStyleRaw) }
    @State private var atBottom = true
    /// Pinch in the feed changes this (0.75-1.8); kept across launches.
    @AppStorage("feedTextScale") private var feedTextScale: Double = 1.0
    @State private var unseenNew = false
    @State private var scrollToken = 0
    @State private var findToken = 0
    @State private var highlightVersion = 0
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
    /// Picked but not sent yet: the user is asked first whether the host keeps them.
    @State private var pendingPhotos: [PhotosPickerItem] = []
    @State private var pendingFiles: [URL] = []
    @State private var askKeep = false
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

    /// The one line of the feed: git (branch, worktree, changes), then the agent's model, context
    /// and limits. Search and refresh are in the session menu; the switch back to the terminal
    /// floats over the right end (see `ActiveTerminalSessionView.modeSwitch`).
    private var feedStrip: some View {
        let git = viewModel.gitContext
        let showsUsage = usage.map { SessionUsageItems.hasContent($0) } ?? false
        return HStack(spacing: 8) {
            if !showsUsage {
                Text(viewModel.feedAgentName).foregroundColor(.secondary)
            }
            if git.isGit, let branch = gitNamesFull ? git.branch : git.shortBranch {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.triangle.branch")
                    Text(branch).lineLimit(gitNamesFull ? 2 : 1).truncationMode(.middle)
                    if git.isWorktree, let wt = gitNamesFull ? git.worktreeName : git.shortWorktree {
                        Image(systemName: "square.stack.3d.down.right")
                        Text(wt).lineLimit(gitNamesFull ? 2 : 1).truncationMode(.middle)
                    }
                    if viewModel.gitDirtyCount > 0 {
                        Text("+\(viewModel.gitDirtyCount)").foregroundColor(.orange)
                    }
                }
                .foregroundColor(.secondary)
                .onTapGesture { withAnimation { gitNamesFull.toggle() } }
            }
            if let usage { SessionUsageItems(monitor: usage) }
            Spacer(minLength: 0)
            if usage?.loading == true || viewModel.isTranscriptLoading { ProgressView().controlSize(.mini) }
            // Room for the terminal switch floating over the right end.
            Color.clear.frame(width: 78, height: 1)
        }
        .font(.system(size: 10, design: .monospaced))
        .padding(.horizontal, 10)
        .frame(minHeight: 22)
        .frame(maxWidth: .infinity)
        .background(.bar)
        .contentShape(Rectangle())
        .onTapGesture { onRefreshUsage?(); Task { await viewModel.loadGitContext() } }
        .contextMenu {
            if git.isGit {
                Button { gitSheet = .branches } label: { Label("Branches", systemImage: "arrow.triangle.branch") }
                Button { gitSheet = .worktrees } label: { Label("Worktrees", systemImage: "square.stack.3d.down.right") }
                Button { gitSheet = .changes } label: { Label("Changes", systemImage: "square.and.arrow.up") }
            }
        }
        .sheet(item: $gitSheet) { sheet in
            switch sheet {
            case .changes: ProjectSheetView(viewModel: viewModel, tab: .changes)
            case .branches: ProjectSheetView(viewModel: viewModel, tab: .branches)
            case .worktrees: GitWorktreesSheetView(viewModel: viewModel)
            }
        }
        .task { await viewModel.loadGitContext() }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Tap to refresh")
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
                    presentation: look
                )
                .background(Color.appBackground)
                .task(id: "\(viewModel.transcriptTurns.count)|\(viewModel.transcriptTurns.last?.content?.count ?? 0)|\(colorScheme == .dark)") {
                    if await FeedHighlightStore.prefetch(viewModel.transcriptTurns, dark: colorScheme == .dark) {
                        highlightVersion += 1
                    }
                }
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

            if let prompt = viewModel.screenPrompt {
                FeedPromptCard(prompt: prompt,
                               pick: { viewModel.answerPrompt(index: $0, toggle: prompt.multi) },
                               finish: { viewModel.finishPrompt(confirm: $0) },
                               tab: { viewModel.moveTab(back: $0) })
            }

            if let status = viewModel.screenStatus {
                Text(status)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 4)
            }

            Divider()

            // Bottom prompt: a multi-line editor with a few control keys above it.
            VStack(spacing: 6) {
                // The system keyboard has none of these; a row that scrolls sideways, in groups.
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(FeedKeys.all) { key in
                            feedKey(key.title) { UIImpactFeedbackGenerator(style: .light).impactOccurred(); viewModel.sendInput(Data(key.bytes)) }
                            if key.endsGroup { Divider().frame(height: 18) }
                        }
                    }
                }
                if !uploads.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(uploads) { u in uploadChip(u) }
                        }
                    }
                }
                HStack(alignment: .bottom, spacing: 8) {
                    Menu {
                        Button { showPhotoPicker = true } label: { Label("Photos & videos", systemImage: "photo.on.rectangle") }
                        Button { showFileImporter = true } label: { Label("Files", systemImage: "folder") }
                    } label: {
                        Image(systemName: "paperclip.circle.fill")
                            .font(.system(size: 30))
                            .foregroundColor(.appAccent)
                    }
                    .disabled(viewModel.sessionHandle == nil || viewModel.state != .connected)
                    TextField("Message…", text: $promptText, axis: .vertical)
                        .lineLimit(1...(editorExpanded ? 16 : 6))
                        .focused($inputFocused)
                        .font(.system(size: 16))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 9)
                    .background(Color.appSecondaryBackground)
                    .cornerRadius(10)
                    .overlay(
                        RoundedRectangle(cornerRadius: 10)
                            .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                    )
                    .overlay(alignment: .topTrailing) {
                        Button { withAnimation { editorExpanded.toggle() } } label: {
                            Image(systemName: editorExpanded
                                ? "arrow.down.right.and.arrow.up.left"
                                : "arrow.up.left.and.arrow.down.right")
                                .font(.system(size: 11))
                                .foregroundColor(.secondary)
                                .padding(8)
                        }
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
                            Image(systemName: "bolt.circle.fill")
                                .font(.system(size: 30))
                                .foregroundColor(canSend ? .orange : .secondary)
                        }
                        .disabled(!canSend)
                        .accessibilityLabel("Send now")
                    }
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
    }

    private func decorated<V: View>(_ content: V) -> some View {
        content
        .photosPicker(isPresented: $showPhotoPicker, selection: $pickedPhotos, maxSelectionCount: 6,
                      matching: .any(of: [.images, .videos]))
        .onChange(of: pickedPhotos) { _, items in
            guard !items.isEmpty else { return }
            pickedPhotos = []
            pendingPhotos += items
            askKeep = true
        }
        .fileImporter(isPresented: $showFileImporter, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            pendingFiles += urls
            askKeep = true
        }
        .confirmationDialog("Keep on the host?", isPresented: $askKeep, titleVisibility: .visible) {
            Button("Keep permanently") { sendPending(permanent: true) }
            Button("Temporary") { sendPending(permanent: false) }
            Button("Cancel", role: .cancel) { pendingPhotos = []; pendingFiles = [] }
        } message: {
            Text("Permanent: ~/.ssh-app/inbox, stays until you delete it. Temporary: the host's temp folder, which the system empties.")
        }
        .onChange(of: viewModel.feedFindRequests) { _, _ in findToken += 1 }
        .onAppear {
            if viewModel.transcriptTurns.isEmpty {
                viewModel.loadTranscript()
            }
            adoptScreenDraft()
        }
        .onChange(of: promptText) { _, _ in
            if adopting { adopting = false } else { scheduleDraftSync() }
        }
        .onChange(of: viewModel.screenDraft) { _, _ in adoptScreenDraft() }
        // Live tail: only while this view is on screen (the task ends with it).
        .task {
            // 3 s while the transcript moves; 5 s, then 10 s when it stands still (each poll is a
            // remote command, a parse and a layout: that is what warms the phone).
            var quiet = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(quiet >= 6 ? 10 : (quiet >= 2 ? 5 : 3)))
                quiet = await viewModel.reloadTranscriptQuietly() ? 0 : quiet + 1
            }
        }
    }

    private func uploadChip(_ u: FeedUpload) -> some View {
        HStack(spacing: 5) {
            switch u.state {
            case .sending: ProgressView().controlSize(.mini)
            case .failed: Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.orange)
            }
            Text(u.failure.map { "\(u.name): \($0)" } ?? u.name)
                .font(.system(size: 11, design: .monospaced))
                .lineLimit(1)
            if u.state == .failed {
                Button { uploads.removeAll { $0.id == u.id } } label: { Image(systemName: "xmark.circle.fill") }
                    .foregroundColor(.secondary)
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(Color.appSecondaryBackground, in: Capsule())
    }

    private func sendPending(permanent: Bool) {
        let photos = pendingPhotos, files = pendingFiles
        pendingPhotos = []
        pendingFiles = []
        for item in photos { Task { await attachPhoto(item, permanent: permanent) } }
        for url in files { Task { await attachFile(url, permanent: permanent) } }
    }

    private func attachPhoto(_ item: PhotosPickerItem, permanent: Bool) async {
        let label = item.supportedContentTypes.first?.preferredFilenameExtension.map { "photo.\($0)" } ?? "photo"
        let id = UUID()
        uploads.append(FeedUpload(id: id, name: label))
        guard let picked = try? await item.loadTransferable(type: PickedFile.self) else {
            fail(id, "could not read")
            return
        }
        defer { try? FileManager.default.removeItem(at: picked.url) }
        await send(id, url: picked.url, name: picked.name, permanent: permanent)
    }

    private func attachFile(_ url: URL, permanent: Bool) async {
        let id = UUID()
        uploads.append(FeedUpload(id: id, name: url.lastPathComponent))
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        await send(id, url: url, name: url.lastPathComponent, permanent: permanent)
    }

    private func send(_ id: UUID, url: URL, name: String, permanent: Bool) async {
        do {
            let path = try await viewModel.uploadAttachment(url, name: name, permanent: permanent)
            uploads.removeAll { $0.id == id }
            // the path goes into the message where the cursor was left: on its own, space-separated
            let sep = promptText.isEmpty || promptText.hasSuffix(" ") || promptText.hasSuffix("\n") ? "" : " "
            promptText += sep + path + " "
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        } catch {
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
                .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
    }

    private func sendPrompt() {
        guard canSend else { return }
        let text = promptText
        promptText = ""
        syncTask?.cancel()
        Task {
            // The box usually holds the text already (it is mirrored as you type): only Enter is
            // missing. A burst of characters is a paste to the agent, so Enter goes separately.
            if await viewModel.syncDraft(text.trimmingCharacters(in: .whitespacesAndNewlines)) {
                try? await Task.sleep(for: .milliseconds(120))
                viewModel.sendInput(Data([0x0d]))
                viewModel.draftWasSent()
            } else {
                // Not mirrored (a menu is up, too long): the old way, text and Enter.
                viewModel.sendInput(Data(Array(FeedSubmit.bytes(for: text).dropLast())))
                try? await Task.sleep(for: .milliseconds(120))
                viewModel.sendInput(Data([0x0d]))
                viewModel.draftWasSent()
            }
            // The transcript catches up through the live tail.
            try? await Task.sleep(for: .seconds(1))
            _ = await viewModel.reloadTranscriptQuietly()
        }
    }

    private func sendImmediately() {
        guard canSend else { return }
        let text = promptText.trimmingCharacters(in: .whitespacesAndNewlines)
        promptText = ""
        syncTask?.cancel()
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        Task {
            if await !viewModel.sendImmediately(text) {
                promptText = text   // not sent: keep the text
            }
            try? await Task.sleep(for: .seconds(1))
            _ = await viewModel.reloadTranscriptQuietly()
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

    /// A draft that is already in the agent's box (typed before the app was closed, or at the
    /// keyboard of the laptop) fills the message field, as long as you have not written anything else.
    private func adoptScreenDraft() {
        guard let adopted = viewModel.adoptableDraft(current: promptText) else { return }
        adopting = true
        promptText = adopted
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
    let finish: (Bool) -> Void
    let tab: (Bool) -> Void

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
                Button { pick(i) } label: {
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
            }
            HStack {
                if prompt.title?.contains("Submit") == true {
                    Button { tab(true) } label: { Label("Back", systemImage: "chevron.left") }.buttonStyle(.bordered)
                    Button { tab(false) } label: { Label("Next", systemImage: "chevron.right") }.buttonStyle(.borderedProminent)
                }
                // in a multi-question form Enter only toggles; "Done" is the next tab (the last one is Submit)
                if prompt.multi && prompt.title?.contains("Submit") != true {
                    Button("Done") { finish(true) }.buttonStyle(.borderedProminent)
                }
                Spacer()
                Button("Esc") { finish(false) }.buttonStyle(.bordered)
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
