import SwiftUI

/// Settings → Appearance: a menu as a grid, four to a row (the quick menu, or the session menu with
/// `sessionActions`). Drag a tile onto another to move it there; in the quick menu a tap shows or
/// hides a tile (Settings itself always stays). The two menus share one saved order.
struct QuickMenuOrderGrid: View {
    var sessionActions = false
    @AppStorage(QuickMenuPrefs.storageKey) private var hiddenRaw: String = ""
    @AppStorage(QuickMenuPrefs.orderKey) private var orderRaw: String = ""
    @AppStorage(MenuGridPrefs.columnsKey(session: false)) private var quickColumns = MenuGridPrefs.defaultColumns
    @AppStorage(MenuGridPrefs.sizeKey(session: false)) private var quickSize = MenuGridPrefs.defaultSize
    @AppStorage(MenuGridPrefs.columnsKey(session: true)) private var sessionColumns = MenuGridPrefs.defaultColumns
    @AppStorage(MenuGridPrefs.sizeKey(session: true)) private var sessionSize = MenuGridPrefs.defaultSize
    @State private var dragging: AppAction?

    private var count: Int { sessionActions ? sessionColumns : quickColumns }
    private var size: Double { sessionActions ? sessionSize : quickSize }
    private var columns: [GridItem] { Array(repeating: GridItem(.flexible(), spacing: 6), count: count) }

    private var order: [AppAction] {
        QuickMenuPrefs.ordered(from: orderRaw).filter { $0.isSessionAction == sessionActions }
    }

    var body: some View {
        Stepper("Tiles per row: \(count)", value: sessionActions ? $sessionColumns : $quickColumns,
                in: MenuGridPrefs.columnRange)
        HStack {
            Text("Icon size")
            Slider(value: sessionActions ? $sessionSize : $quickSize, in: MenuGridPrefs.sizeRange, step: 2)
            Text("\(Int(size))").font(.caption.monospacedDigit()).foregroundColor(.secondary)
        }
        LazyVGrid(columns: columns, spacing: 12) {
            ForEach(order) { action in
                tile(action)
            }
        }
        .padding(.vertical, 6)
        Button("Reset this menu") {
            orderRaw = QuickMenuPrefs.raw(from: QuickMenuPrefs.ordered(from: orderRaw).filter { $0.isSessionAction != sessionActions }
                + QuickMenuPrefs.ordered(from: orderRaw).filter { $0.isSessionAction == sessionActions }.sorted { a, b in
                    AppAction.allCases.firstIndex(of: a)! < AppAction.allCases.firstIndex(of: b)!
                })
            hiddenRaw = QuickMenuPrefs.raw(from: QuickMenuPrefs.hidden(from: hiddenRaw).filter { $0.isSessionAction != sessionActions })
            if sessionActions { sessionColumns = MenuGridPrefs.defaultColumns; sessionSize = MenuGridPrefs.defaultSize }
            else { quickColumns = MenuGridPrefs.defaultColumns; quickSize = MenuGridPrefs.defaultSize }
        }
    }

    private func tile(_ action: AppAction) -> some View {
        let isHidden = QuickMenuPrefs.hidden(from: hiddenRaw).contains(action)
        return VStack(spacing: 4) {
            ZStack {
                Circle().fill(action.tint.opacity(0.18))
                Image(systemName: action.systemImage(isChatMode: false, keyVisible: false))
                    .font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundColor(action.tint)
            }
            .frame(width: size, height: size)
            .overlay(alignment: .topTrailing) {
                Image(systemName: !action.canHide ? "lock.fill" : (isHidden ? "eye.slash.fill" : "eye.fill"))
                    .font(.system(size: 9, weight: .bold))
                    .foregroundColor(.white)
                    .padding(3)
                    .background(Circle().fill(isHidden ? Color.gray : Color.accentColor))
                    .offset(x: 5, y: -3)
            }
            Text(action.title(isChatMode: false))
                .font(.caption2)
                .multilineTextAlignment(.center)
                .lineLimit(2)
        }
        .frame(maxWidth: .infinity)
        .opacity(isHidden ? 0.4 : (dragging == action ? 0.5 : 1))
        .contentShape(Rectangle())
        .onTapGesture {
            if action.canHide { hiddenRaw = QuickMenuPrefs.toggled(action, in: hiddenRaw) }
        }
        .draggable(action.rawValue) {
            Text(action.title(isChatMode: false)).padding(6).glassChip()
        }
        .dropDestination(for: String.self) { items, _ in
            guard let id = items.first, let moving = AppAction(rawValue: id) else { return false }
            orderRaw = QuickMenuPrefs.moved(moving, to: action, in: orderRaw)
            return true
        }
        .accessibilityLabel("\(action.title(isChatMode: false)), \(isHidden ? "hidden" : "shown")")
        .accessibilityHint("Tap to show or hide. Drag onto another tile to move it.")
    }
}
