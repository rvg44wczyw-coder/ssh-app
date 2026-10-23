import SwiftUI

/// Whether the chat gets the whole screen: no navigation bar, a thin tab strip, and the extra keys
/// in a floating quick menu. Off, only in landscape (the default), or always.
enum CompactChatMode: String, CaseIterable, Identifiable {
    case off, landscape, always

    static let storageKey = "compact_chat_mode"
    var id: String { rawValue }

    var title: String {
        switch self {
        case .off: return "Off"
        case .landscape: return "Landscape"
        case .always: return "Always"
        }
    }

    func isCompact(verticalSizeClass: UserInterfaceSizeClass?) -> Bool {
        switch self {
        case .off: return false
        case .landscape: return verticalSizeClass == .compact
        case .always: return true
        }
    }
}

private struct CompactChatKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// True when the compact chat layout is in effect (set by `MainTerminalView`).
    var compactChat: Bool {
        get { self[CompactChatKey.self] }
        set { self[CompactChatKey.self] = newValue }
    }
}

/// A small glass button the person can drag anywhere; it opens the actions the narrow top bar no
/// longer has room for: all sessions, the quick menu and the extra keys.
struct FloatingQuickMenu: View {
    let isChatMode: Bool
    let isConnected: Bool
    let onAllSessions: () -> Void
    let onAction: (AppAction) -> Void
    let onKey: ([UInt8]) -> Void

    @AppStorage("floating_menu_x") private var fx: Double = 0.94
    @AppStorage("floating_menu_y") private var fy: Double = 0.12
    @State private var open = false
    @State private var drag: CGSize = .zero
    private let size: CGFloat = 40

    var body: some View {
        GeometryReader { geo in
            let w = max(geo.size.width - size, 1), h = max(geo.size.height - size, 1)
            let x = min(max(fx * w + drag.width, 0), w), y = min(max(fy * h + drag.height, 0), h)
            Button { open = true } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 16, weight: .semibold))
                    .frame(width: size, height: size)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .glassEffect(.regular.interactive(), in: Circle())
            .accessibilityLabel("Quick menu")
            .offset(x: x, y: y)
            .highPriorityGesture(
                DragGesture(minimumDistance: 6)
                    .onChanged { drag = $0.translation }
                    .onEnded { value in
                        fx = min(max((fx * w + value.translation.width) / w, 0), 1)
                        fy = min(max((fy * h + value.translation.height) / h, 0), 1)
                        drag = .zero
                    }
            )
            .popover(isPresented: $open, arrowEdge: .leading) {
                content.presentationCompactAdaptation(.popover)
            }
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Button {
                    open = false
                    onAllSessions()
                } label: {
                    Label("All sessions", systemImage: "square.grid.2x2")
                }
                if isChatMode {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 46), spacing: 6)], spacing: 6) {
                        ForEach(FeedKeys.all) { key in
                            Button { onKey(key.bytes) } label: {
                                Text(key.title)
                                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                                    .frame(maxWidth: .infinity, minHeight: 30)
                                    .background(Color.secondary.opacity(0.14), in: RoundedRectangle(cornerRadius: 6))
                            }
                            .buttonStyle(.plain)
                            .disabled(!isConnected)
                        }
                    }
                }
                ActionPanelView(isChatMode: isChatMode, keyVisible: false, isConnected: isConnected) {
                    open = false
                    onAction($0)
                }
            }
            .padding(12)
        }
        .frame(minWidth: 320, idealWidth: 360, minHeight: 280, idealHeight: 420)
    }
}
