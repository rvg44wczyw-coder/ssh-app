import SshCoreBridge
import SwiftUI

/// Main screen: every open session as a card with a live preview of its screen. Tap opens the
/// terminal; long press renames or closes; the last card starts a new session.
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
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(sessions) { s in card(s) }
                    addCard
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
            Button { renaming = s; renameText = s.title } label: { Label("Rename", systemImage: "pencil") }
            if sessions.count > 1 {
                Button(role: .destructive) { onClose(s.id) } label: { Label("Close", systemImage: "xmark.circle") }
            }
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
