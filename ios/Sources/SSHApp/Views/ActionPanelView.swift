import SwiftUI

/// Popover opened by the single "More" toolbar button: a grid of labeled tiles. Tap runs the
/// action; press-and-hold shows what it does without running it.
struct ActionPanelView: View {
    enum Scope { case quick, session }
    var scope: Scope = .quick
    let isChatMode: Bool
    let keyVisible: Bool
    let isConnected: Bool
    let onSelect: (AppAction) -> Void

    @Environment(\.dynamicTypeSize) private var typeSize
    @AppStorage(QuickMenuPrefs.storageKey) private var hiddenRaw: String = ""
    @AppStorage(QuickMenuPrefs.orderKey) private var orderRaw: String = ""
    @State private var editing = false
    @State private var hintAction: AppAction? = nil
    @State private var hintToken = 0

    @AppStorage(MenuGridPrefs.columnsKey(session: false)) private var quickColumns = MenuGridPrefs.defaultColumns
    @AppStorage(MenuGridPrefs.sizeKey(session: false)) private var quickSize = MenuGridPrefs.defaultSize
    @AppStorage(MenuGridPrefs.columnsKey(session: true)) private var sessionColumns = MenuGridPrefs.defaultColumns
    @AppStorage(MenuGridPrefs.sizeKey(session: true)) private var sessionSize = MenuGridPrefs.defaultSize

    private var iconSize: CGFloat { CGFloat(scope == .session ? sessionSize : quickSize) }

    /// The panel is as wide as its tiles need, so fewer columns or smaller icons make it narrower and
    /// more or bigger ones wider.
    private var panelWidth: CGFloat {
        let chosen = scope == .session ? sessionColumns : quickColumns
        return MenuGridPrefs.panelWidth(columns: MenuGridPrefs.shownColumns(chosen, accessibility: typeSize.isAccessibilitySize),
                                        iconSize: Double(iconSize))
    }

    private var columns: [GridItem] {
        let chosen = scope == .session ? sessionColumns : quickColumns
        let count = MenuGridPrefs.shownColumns(chosen, accessibility: typeSize.isAccessibilitySize)
        return Array(repeating: GridItem(.flexible(), spacing: 6), count: count)
    }

    var body: some View {
        VStack(spacing: 12) {
            if scope == .quick, editing || !hidden.isEmpty {
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
            LazyVGrid(columns: columns, spacing: 12) {
                ForEach(QuickMenuPrefs.ordered(from: orderRaw).filter { $0.isSessionAction == (scope == .session) && shownWithoutHost($0) && (editing || !$0.onlyInFeed || isChatMode) && (editing || !hidden.contains($0)) }) { action in
                    tile(action)
                }
            }
            if scope == .session {
                hintArea
            } else if editing {
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
        .padding(14)
        .frame(width: panelWidth)
    }

    /// Settings → Terminal → AI Assistant decides: only while connected (default), always, or never.
    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false

    private func shownWithoutHost(_ action: AppAction) -> Bool {
        action != .aiAssistant
            || AiAssistantVisibility.resolved(raw: aiVisibilityRaw, legacyAlways: legacyAlwaysShowAi)
                .isVisible(connected: isConnected)
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
        VStack(spacing: 0) {
            ZStack {
                Circle().fill(action.tint.opacity(isEnabled ? 0.18 : 0.08))
                Image(systemName: action.systemImage(isChatMode: isChatMode, keyVisible: keyVisible))
                    .font(.system(size: iconSize * 0.42, weight: .semibold))
                    .foregroundColor(isEnabled ? action.tint : .secondary)
            }
            .frame(width: iconSize, height: iconSize)
            .overlay(alignment: .topTrailing) {
                if editing {
                    Image(systemName: !action.canHide ? "lock.fill" : (isHidden ? "eye.slash.fill" : "eye.fill"))
                        .font(.system(size: 11, weight: .bold))
                        .foregroundColor(.white)
                        .padding(4)
                        .background(Circle().fill(isHidden ? Color.gray : Color.appAccent))
                        .offset(x: 6, y: -4)
                }
            }
        }
        // Icons only: press and hold says what a tile does (and VoiceOver reads its name).
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
