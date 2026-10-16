import SwiftUI

/// Popover opened by the single "More" toolbar button: a grid of labeled tiles. Tap runs the
/// action; press-and-hold shows what it does without running it.
struct ActionPanelView: View {
    let isChatMode: Bool
    let keyVisible: Bool
    let isConnected: Bool
    let onSelect: (AppAction) -> Void

    @Environment(\.dynamicTypeSize) private var typeSize
    @State private var hintAction: AppAction? = nil
    @State private var hintToken = 0

    private var columns: [GridItem] {
        let count = typeSize.isAccessibilitySize ? 2 : 3
        return Array(repeating: GridItem(.flexible(), spacing: 10), count: count)
    }

    var body: some View {
        VStack(spacing: 12) {
            LazyVGrid(columns: columns, spacing: 14) {
                ForEach(AppAction.allCases) { action in
                    tile(action)
                }
            }
            hintArea
        }
        .padding(16)
        .frame(minWidth: 300, idealWidth: 340)
    }

    private func enabled(_ action: AppAction) -> Bool {
        !action.requiresConnection || isConnected
    }

    @ViewBuilder
    private func tile(_ action: AppAction) -> some View {
        let isEnabled = enabled(action)
        let title = action.title(isChatMode: isChatMode)
        VStack(spacing: 6) {
            ZStack {
                Circle().fill(action.tint.opacity(isEnabled ? 0.18 : 0.08))
                Image(systemName: action.systemImage(isChatMode: isChatMode, keyVisible: keyVisible))
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundColor(isEnabled ? action.tint : .secondary)
            }
            .frame(width: 48, height: 48)
            Text(title)
                .font(.caption)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .foregroundColor(isEnabled ? .primary : .secondary)
        }
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
        .onTapGesture {
            if isEnabled {
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
        .accessibilityLabel(title)
        .accessibilityHint(isEnabled ? action.hint : "Connect to a host first. " + action.hint)
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
        if isEnabled { onSelect(action) } else { show(hint: action) }
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
