import SwiftUI

/// Popover opened by the single "More" toolbar button: a grid of labeled tiles. Tap runs the
/// action; press-and-hold shows what it does without running it.
struct ActionPanelView: View {
    let isChatMode: Bool
    let keyVisible: Bool
    let isConnected: Bool
    let onSelect: (AppAction) -> Void

    @Environment(\.dynamicTypeSize) private var typeSize
    @AppStorage(QuickMenuPrefs.storageKey) private var hiddenRaw: String = ""
    @State private var editing = false
    @State private var hintAction: AppAction? = nil
    @State private var hintToken = 0

    private var columns: [GridItem] {
        let count = typeSize.isAccessibilitySize ? 2 : 3
        return Array(repeating: GridItem(.flexible(), spacing: 10), count: count)
    }

    var body: some View {
        VStack(spacing: 12) {
            if editing || !hidden.isEmpty {
                HStack {
                    Text(editing ? "Tap a tile to show or hide it" : "\(hidden.count) hidden")
                        .font(.caption).foregroundColor(.secondary)
                    Spacer()
                    if editing, !hidden.isEmpty {
                        Button("Show all") { hiddenRaw = "" }.font(.caption)
                    }
                    Button(editing ? "Done" : "Edit") { withAnimation { editing.toggle() } }
                        .font(.caption.bold())
                }
            }
            LazyVGrid(columns: columns, spacing: 14) {
                ForEach(editing ? AppAction.allCases : AppAction.allCases.filter { !hidden.contains($0) && shownWithoutHost($0) }) { action in
                    tile(action)
                }
            }
            if editing {
                Text("Settings always stays, so you can get everything back.")
                    .font(.caption2).foregroundColor(.secondary)
            } else {
                hintArea
                if hidden.isEmpty {
                    Button("Customize menu") { withAnimation { editing = true } }
                        .font(.caption2)
                }
            }
        }
        .padding(16)
        .frame(minWidth: 300, idealWidth: 340)
    }

    /// The AI assistant only works through a live host, so without one its tile is left out
    /// (Settings → Terminal → "Always show AI Assistant" brings it back).
    @AppStorage("always_show_ai_assistant") private var alwaysShowAi: Bool = false

    private func shownWithoutHost(_ action: AppAction) -> Bool {
        action != .aiAssistant || isConnected || alwaysShowAi
    }

    private var hidden: Set<AppAction> { QuickMenuPrefs.hidden(from: hiddenRaw) }

    private func enabled(_ action: AppAction) -> Bool {
        !action.requiresConnection || isConnected
    }

    @ViewBuilder
    private func tile(_ action: AppAction) -> some View {
        let isHidden = hidden.contains(action)
        // While customizing every tile is a toggle, never an action.
        let isEnabled = editing ? action.canHide : enabled(action)
        let title = action.title(isChatMode: isChatMode)
        VStack(spacing: 6) {
            ZStack {
                Circle().fill(action.tint.opacity(isEnabled ? 0.18 : 0.08))
                Image(systemName: action.systemImage(isChatMode: isChatMode, keyVisible: keyVisible))
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundColor(isEnabled ? action.tint : .secondary)
            }
            .frame(width: 48, height: 48)
            .overlay(alignment: .topTrailing) {
                if editing {
                    Image(systemName: !action.canHide ? "lock.fill" : (isHidden ? "eye.slash.fill" : "eye.fill"))
                        .font(.system(size: 11, weight: .bold))
                        .foregroundColor(.white)
                        .padding(4)
                        .background(Circle().fill(isHidden ? Color.gray : Color.accentColor))
                        .offset(x: 6, y: -4)
                }
            }
            Text(title)
                .font(.caption)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .foregroundColor(isEnabled ? .primary : .secondary)
        }
        .frame(maxWidth: .infinity)
        .opacity(editing && isHidden ? 0.4 : 1)
        .contentShape(Rectangle())
        .onTapGesture {
            if editing {
                if action.canHide { hiddenRaw = QuickMenuPrefs.toggled(action, in: hiddenRaw) }
            } else if isEnabled {
                onSelect(action)
            } else {
                show(hint: action)
            }
        }
        .onLongPressGesture(minimumDuration: 0.4) {
            show(hint: action)
        }
        // Hardware keyboard: Tab moves between tiles, Return/Space activates (or explains, if disabled).
        .focusable()
        .onKeyPress(.return) { activate(action, isEnabled: isEnabled) }
        .onKeyPress(.space) { activate(action, isEnabled: isEnabled) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(editing ? "\(title), \(isHidden ? "hidden" : "shown")" : title)
        .accessibilityHint(editing ? (action.canHide ? "Double tap to \(isHidden ? "show" : "hide") this tile" : "Always shown") : isEnabled ? action.hint : "Connect to a host first. " + action.hint)
        .accessibilityAddTraits(.isButton)
    }

    @ViewBuilder
    private var hintArea: some View {
        if let action = hintAction {
            VStack(alignment: .leading, spacing: 4) {
                Text(action.title(isChatMode: isChatMode)).font(.subheadline.bold())
                Text(enabled(action) ? action.hint : "Connect to a host first. " + action.hint)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 10).fill(action.tint.opacity(0.14)))
            .transition(.opacity)
        } else {
            Text("Press and hold an icon to see what it does")
                .font(.caption2)
                .foregroundColor(.secondary)
        }
    }

    private func activate(_ action: AppAction, isEnabled: Bool) -> KeyPress.Result {
        if editing {
            if action.canHide { hiddenRaw = QuickMenuPrefs.toggled(action, in: hiddenRaw) }
        } else if isEnabled { onSelect(action) } else { show(hint: action) }
        return .handled
    }

    private func show(hint action: AppAction) {
        #if os(iOS)
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        #endif
        withAnimation(.easeInOut(duration: 0.15)) { hintAction = action }
        hintToken += 1
        let token = hintToken
        Task {
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            if token == hintToken {
                withAnimation(.easeInOut(duration: 0.15)) { hintAction = nil }
            }
        }
    }
}
