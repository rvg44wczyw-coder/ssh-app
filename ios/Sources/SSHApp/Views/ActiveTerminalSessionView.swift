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

    /// The switch from the terminal to the feed: a still icon in the top right corner of the
    /// terminal. (In the feed the way back is an icon in the feed's own status line.)
    private func modeSwitch(_ title: String, _ icon: String, hint: String) -> some View {
        Button { viewModel.toggleChatMode() } label: {
            Image(systemName: icon)
                .font(.system(size: 13, weight: .semibold))
                .frame(width: 34, height: 28)
                .background(Color.appSecondaryBackground.opacity(0.92), in: Capsule())
                .overlay(Capsule().stroke(Color.secondary.opacity(0.25), lineWidth: 0.5))
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .padding(.top, 6)
        .padding(.trailing, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
        .accessibilityLabel(title)
        .accessibilityHint(hint)
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
                if !viewModel.isChatMode, viewModel.state == .connected, let agent = modelMenuAgent, agent != "ollama" {
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
