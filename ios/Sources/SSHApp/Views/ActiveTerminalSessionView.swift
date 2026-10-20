import SwiftUI
import SshCoreBridge

struct ActiveTerminalSessionView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Binding var isCtrlLocked: Bool
    let scheme: TerminalScheme
    let onRetry: () -> Void
    let onOpenAiAssistant: () -> Void
    let onOpenSnippets: () -> Void
    /// Shown in the feed's one line; `nil` when the usage strip is switched off.
    var usage: SessionUsageMonitor? = nil
    var onRefreshUsage: (() -> Void)? = nil
    /// The agent whose models the quick-command menus list, from the tab's agent kind.
    @State private var detectedMenuAgent: String?

    private var modelMenuAgent: String? {
        switch viewModel.session.agentType {
        case .claudeCode: return "claude"
        case .antigravity: return "antigravity"
        case .localLlm: return "ollama"
        case .customShell: return detectedMenuAgent
        }
    }

    /// Re-asks the host what runs in a plain session's pane (the user may start `claude` or `agy`
    /// by hand), every 15 s while this screen is visible and connected. Typed sessions skip it.
    private func watchPaneAgent() async {
        guard viewModel.session.agentType == .customShell else { return }
        while !Task.isCancelled {
            if viewModel.state == .connected {
                let name = await viewModel.currentPaneContext().agent
                let mapped: String? = switch name {
                case "claude": "claude"
                case "agy", "antigravity": "antigravity"
                case "ollama": "ollama"
                default: nil
                }
                if mapped != detectedMenuAgent { detectedMenuAgent = mapped }
            }
            try? await Task.sleep(for: .seconds(15))
        }
    }

    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false

    @AppStorage(ModeSwitchPrefs.xKey) private var switchX = ModeSwitchPrefs.defaultX
    @AppStorage(ModeSwitchPrefs.yKey) private var switchY = ModeSwitchPrefs.defaultY
    @AppStorage(ModeSwitchPrefs.opacityKey) private var switchOpacity = ModeSwitchPrefs.defaultOpacity
    @State private var switchDrag: CGSize = .zero

    /// The switch between terminal and feed: the same button on both screens, in the same place.
    /// Drag it anywhere, long press puts it back; its opacity is in Settings → Appearance.
    private func modeSwitch(_ title: String, _ icon: String, hint: String) -> some View {
        GeometryReader { geo in
            let w = geo.size.width, h = geo.size.height
            Button { viewModel.toggleChatMode() } label: {
                Label(title, systemImage: icon)
                    .font(.system(size: 12, weight: .semibold))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.appSecondaryBackground.opacity(switchOpacity), in: Capsule())
                    .overlay(Capsule().stroke(Color.secondary.opacity(0.25 * switchOpacity), lineWidth: 0.5))
            }
            .buttonStyle(.plain)
            .fixedSize()
            .position(
                x: min(max(switchX * w + switchDrag.width, 40), max(40, w - 40)),
                y: min(max(switchY * h + switchDrag.height, 16), max(16, h - 16))
            )
            .simultaneousGesture(
                DragGesture(minimumDistance: 8)
                    .onChanged { switchDrag = $0.translation }
                    .onEnded { d in
                        if w > 0, h > 0 {
                            switchX = min(max(switchX + d.translation.width / w, 0.05), 0.95)
                            switchY = min(max(switchY + d.translation.height / h, 0.01), 0.99)
                        }
                        switchDrag = .zero
                    }
            )
            .contextMenu {
                Button {
                    withAnimation { switchX = ModeSwitchPrefs.defaultX; switchY = ModeSwitchPrefs.defaultY }
                } label: { Label("Back to the top right", systemImage: "arrow.up.right.square") }
            }
            .accessibilityHint(hint)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            ZStack(alignment: .top) {
                if viewModel.isChatMode {
                    StructuredTranscriptView(viewModel: viewModel, usage: usage, onRefreshUsage: onRefreshUsage)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    #if canImport(UIKit)
                    TerminalContainerView(
                        viewModel: viewModel,
                        isCtrlLocked: $isCtrlLocked,
                        scheme: scheme
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    #else
                    Text("SwiftTerm rendering is native on iOS")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    #endif
                }

                // The switch between terminal and feed: one place on both screens. An agent runs here
                // (also one started by hand in a plain tab): offer the feed.
                if viewModel.isChatMode {
                    modeSwitch("Terminal", "terminal.fill", hint: "Switch this tab back to the raw terminal")
                } else if viewModel.state == .connected, let agent = modelMenuAgent, agent != "ollama" {
                    modeSwitch("Feed", "text.bubble", hint: "Show this agent's conversation as a readable feed")
                }

                // Top Connection Status Banner
                switch viewModel.state {
                case .connecting, .reconnecting:
                    HStack(spacing: 8) {
                        ProgressView()
                            .progressViewStyle(CircularProgressViewStyle(tint: .white))
                            .scaleEffect(0.8)
                        Text(viewModel.state == .reconnecting ? "Reconnecting to tmux..." : "Connecting to host...")
                            .font(.system(size: 13, weight: .medium))
                            .foregroundColor(.white)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.appAccent.opacity(0.85))
                    .cornerRadius(20)
                    .padding(.top, 8)
                    .shadow(radius: 4)

                case .failed(let reason):
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundColor(.yellow)
                        Text(reason)
                            .font(.system(size: 12))
                            .lineLimit(2)
                            .foregroundColor(.white)
                        Button(action: onRetry) {
                            Text("Retry")
                                .font(.system(size: 12, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.white.opacity(0.25))
                                .cornerRadius(6)
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.red.opacity(0.9))
                    .cornerRadius(12)
                    .padding(.top, 8)
                    .padding(.horizontal, 16)
                    .shadow(radius: 4)

                case .disconnected:
                    HStack(spacing: 8) {
                        Image(systemName: "bolt.slash.fill")
                            .foregroundColor(.secondary)
                        Text("Disconnected")
                            .font(.system(size: 12))
                            .foregroundColor(.white)
                        Button(action: onRetry) {
                            Text("Connect")
                                .font(.system(size: 12, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.appAccent)
                                .cornerRadius(6)
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Color(white: 0.15).opacity(0.9))
                    .cornerRadius(20)
                    .padding(.top, 8)

                case .connected:
                    EmptyView()
                }
            }

            #if canImport(UIKit)
            if !viewModel.isChatMode {
                KeyboardAccessoryView(
                    onSendBytes: { bytes in
                        viewModel.sendInput(Data(bytes))
                    },
                    onAiAssistantTap: AiAssistantVisibility
                        .resolved(raw: aiVisibilityRaw, legacyAlways: legacyAlwaysShowAi)
                        .isVisible(connected: true) ? { onOpenAiAssistant() } : nil,
                    onSnippetsTap: {
                        onOpenSnippets()
                    },
                    menuAgent: modelMenuAgent,
                    menuScope: viewModel.session.serverId?.uuidString ?? "",
                    menuReady: viewModel.state == .connected && viewModel.sessionHandle != nil,
                    onPaste: { viewModel.pasteHandler?() },
                    loadModels: { agent in
                        guard let handle = viewModel.sessionHandle, viewModel.state == .connected,
                            let pin = await viewModel.helperPin()
                        else { return [] }
                        let list = (try? await handle.fetchAgentModels(agent: agent, expectedSha256: pin)) ?? []
                        return list.map { (id: $0.id, label: $0.label) }
                    }
                )
                .task(id: viewModel.state == .connected) { await watchPaneAgent() }
            }
            #endif
        }
    }
}
