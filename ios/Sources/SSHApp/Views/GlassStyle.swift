import SwiftUI

/// The glass vocabulary of the app (Liquid Glass, iOS 27). Standard controls (navigation bars,
/// toolbars, sheets, lists, `.glass` buttons) are glass by themselves; these helpers cover the
/// custom surfaces, so cards, chips and bars look and round the same everywhere. Terminal and feed
/// text keep a solid background: glass under running text hurts reading.
enum GlassMetrics {
    static let card: CGFloat = 18
    static let tile: CGFloat = 8
}

extension View {
    /// A glass card behind grouped content. Padding stays with the caller.
    func glassCard(cornerRadius: CGFloat = GlassMetrics.card, tint: Color? = nil, interactive: Bool = false) -> some View {
        glassEffect(Self.glass(tint: tint, interactive: interactive), in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
    }

    /// A glass capsule for a label, badge or small control.
    func glassChip(tint: Color? = nil, interactive: Bool = false) -> some View {
        glassEffect(Self.glass(tint: tint, interactive: interactive), in: Capsule())
    }

    /// A full-width glass bar (the strip above the keyboard, a bottom bar).
    func glassBar() -> some View {
        glassEffect(.regular, in: Rectangle())
    }

    /// Lets this element morph into its neighbours inside a `GlassEffectContainer` (tabs, chips,
    /// the floating menu): give elements that stand for the same thing the same id.
    func glassMorph(_ id: some Hashable & Sendable, in namespace: Namespace.ID) -> some View {
        glassEffectID(id, in: namespace)
    }

    private static func glass(tint: Color?, interactive: Bool) -> Glass {
        var g = Glass.regular
        if let tint { g = g.tint(tint) }
        if interactive { g = g.interactive() }
        return g
    }
}

/// A round glass button face for the prompt row (attach, commands, send). `tint` colours the
/// glass for the primary action; without it the glass stays clear and the symbol takes `symbolColor`.
struct GlassRoundIcon: View {
    let symbol: String
    var tint: Color?
    var symbolColor: Color = .primary
    var size: CGFloat = 38
    /// Shows this text instead of the symbol (the slash of the commands button).
    var text: String?
    /// Off inside a `Menu` label: interactive glass takes the touch itself and the menu never opens.
    var interactive = true

    @ViewBuilder
    private var face: some View {
        if let text { Text(verbatim: text) } else { Image(systemName: symbol) }
    }

    private static func glass(tint: Color?, interactive: Bool) -> Glass {
        var g = Glass.regular
        if let tint { g = g.tint(tint) }
        return interactive ? g.interactive() : g
    }

    var body: some View {
        face
            .font(.system(size: size * 0.45, weight: .semibold, design: text == nil ? .default : .monospaced))
            .foregroundStyle(tint == nil ? symbolColor : .white)
            .frame(width: size, height: size)
            .glassEffect(Self.glass(tint: tint, interactive: interactive), in: Circle())
            .contentShape(Circle())
    }
}

/// The coloured tile in front of a settings row.
struct GlassIconTile: View {
    let symbol: String
    var tint: Color = .appAccent

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 30, height: 30)
            .glassEffect(.regular.tint(tint), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// A settings row label: glass tile and title.
struct SettingsLabel: View {
    let title: LocalizedStringKey
    let symbol: String
    var tint: Color = .appAccent

    init(_ title: LocalizedStringKey, _ symbol: String, tint: Color = .appAccent) {
        self.title = title
        self.symbol = symbol
        self.tint = tint
    }

    var body: some View {
        Label {
            Text(title)
        } icon: {
            GlassIconTile(symbol: symbol, tint: tint)
        }
    }
}

/// Long help behind an "i" button instead of a footer that fills the screen.
struct InfoButton: View {
    let text: LocalizedStringKey
    @State private var shown = false

    init(_ text: LocalizedStringKey) { self.text = text }

    var body: some View {
        Button { shown = true } label: { Image(systemName: "info.circle") }
            .buttonStyle(.borderless)
            .popover(isPresented: $shown) {
                Text(text)
                    .font(.callout)
                    .padding()
                    .frame(maxWidth: 320)
                    .presentationCompactAdaptation(.popover)
            }
            .accessibilityLabel("Help")
    }
}
