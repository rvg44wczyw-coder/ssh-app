import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

public struct KeyboardAccessoryView: View {
    public let onSendBytes: ([UInt8]) -> Void
    public let onAiAssistantTap: (() -> Void)?
    public let onSnippetsTap: (() -> Void)?
    /// Agent of the active tab (`claude`, `antigravity`, `ollama`) and a way to ask the host for its
    /// models; both nil when there is no agent or no connection.
    public let menuAgent: String?
    /// Tells servers apart in the model-list cache (the lists differ per host).
    public let menuScope: String
    /// False while the session is not connected; the lists are fetched once it turns true.
    public let menuReady: Bool
    /// Pastes the clipboard through the terminal (bracketed paste when the program wants it).
    public let onPaste: (() -> Void)?
    public let loadModels: ((String) async -> [(id: String, label: String)])?
    @State private var models: [(id: String, label: String)] = []
    @State private var modelsLoading = false
    @ObservedObject private var store = QuickCommandStore.shared
    @State private var showEditor = false
    @State private var pendingClear: QuickCommand?

    public init(
        onSendBytes: @escaping ([UInt8]) -> Void,
        onAiAssistantTap: (() -> Void)? = nil,
        onSnippetsTap: (() -> Void)? = nil,
        menuAgent: String? = nil,
        menuScope: String = "",
        menuReady: Bool = true,
        onPaste: (() -> Void)? = nil,
        loadModels: ((String) async -> [(id: String, label: String)])? = nil
    ) {
        self.menuAgent = menuAgent
        self.menuScope = menuScope
        self.menuReady = menuReady
        self.onPaste = onPaste
        self.loadModels = loadModels
        self.onSendBytes = onSendBytes
        self.onAiAssistantTap = onAiAssistantTap
        self.onSnippetsTap = onSnippetsTap
    }

    /// Esc, Ctrl, Tab, arrows and symbols are already on the system keyboard bar; this bar only
    /// carries what that one lacks: the assistant, snippets, Ctrl-C, y/n and the user's own
    /// command buttons.
    public var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                if let onAiTap = onAiAssistantTap {
                    AccessoryButton(label: "🪄") {
                        triggerHaptic()
                        onAiTap()
                    }
                }
                if let onSnippets = onSnippetsTap {
                    AccessoryButton(label: "⚡") {
                        triggerHaptic()
                        onSnippets()
                    }
                }
                AccessoryButton(label: "^C") {
                    triggerHaptic()
                    onSendBytes([0x03])
                }
                AccessoryButton(label: "✓ y⏎") {
                    triggerHaptic()
                    onSendBytes([0x79, 0x0D])
                }
                AccessoryButton(label: "✗ n⏎") {
                    triggerHaptic()
                    onSendBytes([0x6E, 0x0D])
                }

                Divider().frame(height: 24)

                ForEach(store.commands) { cmd in
                    AccessoryButton(label: cmd.label, hasMenu: cmd.menu != nil, action: {
                        triggerHaptic()
                        if cmd.confirm {
                            pendingClear = cmd
                        } else if let onPaste, cmd.text.trimmingCharacters(in: .whitespaces) == QuickCommand.clipboardToken {
                            onPaste()
                        } else if let bytes = cmd.resolvedBytes() {
                            onSendBytes(bytes)
                        }
                    }, menu: {
                        if let menu = cmd.menu { choices(of: cmd, menu) }
                    })
                }
                AccessoryButton(label: "✎") {
                    triggerHaptic()
                    showEditor = true
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
        .background(Color.appSecondaryBackground)
        .task(id: "\(menuScope)|\(menuAgent ?? "")|\(menuReady)") { await prefetchModels() }
        .sheet(isPresented: $showEditor) { QuickCommandsEditorView() }
        .confirmationDialog(
            "Run \(pendingClear?.label ?? "")?",
            isPresented: Binding(get: { pendingClear != nil }, set: { if !$0 { pendingClear = nil } }),
            titleVisibility: .visible,
            presenting: pendingClear
        ) { cmd in
            Button("Run \(cmd.label)", role: .destructive) {
                if let bytes = cmd.resolvedBytes() { onSendBytes(bytes) }
            }
            Button("Cancel", role: .cancel) {}
        } message: { cmd in
            Text("Send \"\(cmd.text)\" to the session?")
        }
    }

    /// The items of a command's long-press menu.
    @ViewBuilder
    private func choices(of cmd: QuickCommand, _ menu: QuickCommandMenu) -> some View {
        switch menu.source {
        case .fixed:
            let fixed = menu.fixedChoices
            if fixed.isEmpty { Text("No choices set") }
            ForEach(Array(fixed.enumerated()), id: \.offset) { _, c in
                Button(c.label) { send(cmd, c.value) }
            }
        case .agentModels:
            if menuAgent == nil {
                Text("Open a Claude, Antigravity or Ollama tab")
            } else if models.isEmpty {
                Text(modelsLoading ? "Loading models…" : "No models found on the host")
            } else {
                ForEach(Array(models.enumerated()), id: \.offset) { _, m in
                    Button(m.label) { send(cmd, m.id) }
                }
            }
        }
    }

    private func send(_ cmd: QuickCommand, _ value: String) {
        triggerHaptic()
        if let bytes = cmd.bytes(choosing: value) { onSendBytes(bytes) }
    }

    /// Asks the host once per agent (cached for hours) when a command uses model lists.
    private func prefetchModels() async {
        models = []
        guard menuReady, let agent = menuAgent, let loadModels,
            store.commands.contains(where: { $0.menu?.source == .agentModels })
        else { return }
        let cacheKey = "\(menuScope)|\(agent)"
        if let hit = ModelListCache.shared.cached(cacheKey) { models = hit; return }
        modelsLoading = true
        let fresh = await loadModels(agent)
        modelsLoading = false
        if !fresh.isEmpty { ModelListCache.shared.store(cacheKey, fresh) }
        models = fresh
    }

    private func triggerHaptic() {
        #if canImport(UIKit)
        let generator = UIImpactFeedbackGenerator(style: .light)
        generator.prepare()
        generator.impactOccurred()
        #endif
    }
}

struct AccessoryButton<MenuContent: View>: View {
    let label: String
    var hasMenu = false
    let action: () -> Void
    @ViewBuilder var menu: () -> MenuContent

    var body: some View {
        let button = Button(action: action) {
            HStack(spacing: 3) {
                Text(label)
                if hasMenu {
                    Image(systemName: "chevron.up").font(.system(size: 7, weight: .bold)).foregroundColor(.secondary)
                }
            }
            .font(.system(size: 13, weight: .semibold, design: .monospaced))
            .foregroundColor(.primary)
            .frame(minWidth: 36, minHeight: 34)
            .padding(.horizontal, 6)
            .background(Color.appTertiaryBackground)
            .cornerRadius(6)
            .shadow(color: Color.black.opacity(0.1), radius: 1, y: 1)
        }
        if hasMenu {
            button.contextMenu { menu() }
        } else {
            button
        }
    }
}

extension AccessoryButton where MenuContent == EmptyView {
    init(label: String, action: @escaping () -> Void) {
        self.init(label: label, action: action, menu: { EmptyView() })
    }
}
