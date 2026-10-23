import SshCoreBridge
import SwiftUI

/// Main screen: every open session as a card with a live preview of its screen. Tap opens the
/// terminal; long press opens the same session menu as the tabs; the last card starts a new session.
struct SessionsHomeView: View {
    let sessions: [TabSession]
    let states: [UUID: SessionState]
    let viewModel: (UUID) -> TerminalSessionViewModel?
    let serverName: (TabSession) -> String?
    let icon: (TabSession) -> (symbol: String, color: Color?)
    let scheme: (TabSession) -> TerminalScheme
    let presets: [(id: String, title: String, detail: String)]
    let onOpen: (UUID) -> Void
    let onAdd: () -> Void
    let onLaunchPreset: (String) -> Void
    let onRename: (UUID, String) -> Void
    let onClose: (UUID) -> Void
    let isChatMode: (TabSession) -> Bool
    let attention: (TabSession) -> SessionAttention?
    /// Symbol for what a waiting request is about (a command, an edit, a read ...).
    var requestSymbol: (TabSession) -> String? = { _ in nil }
    let onRestart: (UUID) -> Void
    let onSessionAction: (UUID, AppAction) -> Void
    let onSetScheme: (UUID, String?) -> Void
    /// Assistants that have history, shown as cards after the sessions.
    let assistants: [AssistantSession]
    let assistantServerName: (UUID) -> String
    let onOpenAssistant: (UUID) -> Void
    let onClearAssistant: (UUID) -> Void

    /// Servers in the order they are shown (every server, also one without sessions).
    var servers: [(id: UUID, name: String)] = []
    /// The server a session belongs to (a session without one runs on the active server).
    var serverOf: (TabSession) -> UUID? = { $0.serverId }
    /// Opens a full screen of the server (containers, processes, host monitor).
    var onServerScreen: (UUID, HostScreen) -> Void = { _, _ in }

    @State private var renaming: TabSession?
    @State private var renameText = ""
    private let columns = [GridItem(.adaptive(minimum: 160), spacing: 12)]

    private func stateColor(_ s: SessionState) -> Color {
        switch s {
        case .connected: return .green
        case .connecting, .reconnecting: return .yellow
        case .failed: return .red
        case .disconnected: return .gray
        }
    }

    var body: some View {
        ScrollView {
            // Previews are re-rendered every few seconds while this screen is visible.
            TimelineView(.periodic(from: .now, by: 3)) { _ in
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(servers, id: \.id) { server in
                        serverSection(server)
                    }
                    let orphans = sessions.filter { s in !servers.contains(where: { $0.id == serverOf(s) }) }
                    if !orphans.isEmpty {
                        LazyVGrid(columns: columns, spacing: 12) { ForEach(orphans) { s in card(s) } }
                    }
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(assistants) { a in
                        AssistantCard(assistant: a, serverName: assistantServerName(a.serverId),
                                      onOpen: { onOpenAssistant(a.serverId) }, onClear: { onClearAssistant(a.serverId) })
                    }
                    addCard
                }
                }
                .padding(12)
            }
        }
        .background(Color.appBackground)
        .alert("Rename session", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $renameText)
            Button("Cancel", role: .cancel) {}
            Button("Save") { if let r = renaming { onRename(r.id, renameText) } }
        }
    }

    /// A server's sessions with the screens that belong to the server, not to one session.
    @ViewBuilder
    private func serverSection(_ server: (id: UUID, name: String)) -> some View {
        let mine = sessions.filter { serverOf($0) == server.id }
        let connected = mine.contains { (states[$0.id] ?? .disconnected) == .connected }
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Circle().fill(connected ? Color.green : Color.gray).frame(width: 8, height: 8)
                Text(server.name).font(.headline).lineLimit(1)
                Spacer(minLength: 0)
            }
            GlassEffectContainer(spacing: 8) {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        serverButton("Containers", "shippingbox.fill", .docker, server.id, enabled: connected)
                        serverButton("Processes", "list.bullet.rectangle", .processes, server.id, enabled: connected)
                        serverButton("Monitor", "gauge.with.dots.needle.67percent", .host, server.id, enabled: connected)
                    }
                }
            }
            if !connected {
                Text("Open a session on this server to use its screens.")
                    .font(.caption2).foregroundColor(.secondary)
            }
            if !mine.isEmpty {
                LazyVGrid(columns: columns, spacing: 12) { ForEach(mine) { s in card(s) } }
            }
        }
    }

    private func serverButton(_ title: String, _ symbol: String, _ screen: HostScreen, _ id: UUID, enabled: Bool) -> some View {
        Button { onServerScreen(id, screen) } label: {
            Label(title, systemImage: symbol).font(.subheadline.weight(.medium)).padding(.horizontal, 12).padding(.vertical, 7)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: Capsule())
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.45)
    }

    @ViewBuilder
    private func card(_ s: TabSession) -> some View {
        let sch = scheme(s)
        let ic = icon(s)
        let state = states[s.id] ?? .disconnected
        let lines = viewModel(s.id).map { SessionPreviewCache.shared.lines(for: $0, maxLines: 7) } ?? []
        Button { onOpen(s.id) } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    ProfileIconView(ref: ic.symbol, size: 13).foregroundColor(ic.color ?? .appAccent)
                    Text(s.title).font(.subheadline.bold()).lineLimit(1).foregroundColor(.primary)
                    Spacer(minLength: 0)
                    if let a = attention(s) {
                        if a == .approval, let sym = requestSymbol(s) {
                            Image(systemName: sym).font(.system(size: 12, weight: .semibold)).foregroundColor(.secondary)
                        }
                        SessionAttentionBadge(attention: a, size: 13)
                    }
                    if let vm = viewModel(s.id), vm.unreadCount > 0 {
                        Label("\(vm.unreadCount)", systemImage: "bubble.left.fill")
                            .font(.caption2.weight(.semibold)).foregroundColor(.appAccent)
                            .accessibilityLabel("\(vm.unreadCount) unread")
                    }
                    Circle().fill(stateColor(state)).frame(width: 8, height: 8)
                }
                Text([serverName(s), s.sessionName].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption2).foregroundColor(.secondary).lineLimit(1)
                ZStack(alignment: .topLeading) {
                    RoundedRectangle(cornerRadius: 8).fill(Color(hex: sch.background))
                    if lines.isEmpty {
                        Text(state == .connected ? "No output yet" : "Not connected")
                            .font(.system(size: 9, design: .monospaced))
                            .foregroundColor(Color(hex: sch.foreground).opacity(0.5))
                            .padding(6)
                    } else {
                        Text(verbatim: lines.joined(separator: "\n"))
                            .font(.system(size: 8, design: .monospaced))
                            .foregroundColor(Color(hex: sch.foreground))
                            .lineLimit(7)
                            .multilineTextAlignment(.leading)
                            .padding(6)
                    }
                }
                .frame(height: 96)
                .clipped()
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 14).fill(Color.appSecondaryBackground))
        }
        .buttonStyle(.plain)
        .contextMenu {
            SessionMenuItems(
                session: s,
                isConnected: states[s.id] == .connected,
                isChatMode: isChatMode(s),
                canClose: sessions.count > 1,
                onAction: { onSessionAction(s.id, $0) },
                onScheme: { onSetScheme(s.id, $0) },
                onRename: { renaming = s; renameText = s.title },
                onRestart: s.profileId == nil && s.command == nil ? nil : { onRestart(s.id) },
                onClose: { onClose(s.id) }
            )
        }
        .accessibilityLabel("\(s.title), \(serverName(s) ?? "")")
        .accessibilityHint("Opens this session")
    }

    private var addCard: some View {
        Menu {
            ForEach(presets, id: \.id) { p in
                Button { onLaunchPreset(p.id) } label: {
                    Text(p.detail.isEmpty ? p.title : "\(p.title) — \(p.detail)")
                }
            }
            Button { onAdd() } label: { Label("New terminal", systemImage: "plus") }
        } label: {
            VStack(spacing: 8) {
                Image(systemName: "plus").font(.system(size: 26, weight: .semibold)).foregroundColor(.appAccent)
                Text("New session").font(.caption).foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 170)
            .background(RoundedRectangle(cornerRadius: 14).strokeBorder(Color.appAccent.opacity(0.5), style: StrokeStyle(lineWidth: 1.5, dash: [6])))
        } primaryAction: {
            onAdd()
        }
    }
}

/// The AI assistant of one server as a card in the sessions grid: opens the assistant screen.
private struct AssistantCard: View {
    @ObservedObject var assistant: AssistantSession
    let serverName: String
    let onOpen: () -> Void
    let onClear: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 6) {
                    Image(systemName: "wand.and.stars").foregroundColor(.purple)
                    Text("AI · \(serverName)").font(.system(size: 13, weight: .semibold)).lineLimit(1)
                    Spacer(minLength: 0)
                    if assistant.isChatLoading || assistant.isAiLoading { ProgressView().controlSize(.mini) }
                }
                Text(assistant.preview.isEmpty ? "Ask the host model" : assistant.preview)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.secondary)
                    .lineLimit(6)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                Spacer(minLength: 0)
            }
            .padding(10)
            .frame(maxWidth: .infinity, minHeight: 170, alignment: .topLeading)
            .background(RoundedRectangle(cornerRadius: 14).fill(Color.appSecondaryBackground))
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button(role: .destructive, action: onClear) { Label("Clear chat", systemImage: "trash") }
        }
        .accessibilityLabel("AI assistant, \(serverName)")
        .accessibilityHint("Opens the assistant. History is kept in memory only.")
    }
}
