import SwiftUI
import SshCoreBridge

#if canImport(UIKit)
import UIKit
#endif

// MARK: - Accent colour

/// The app's accent colour (buttons, selected chips, links). Stored as `app_accent`.
public enum AppAccent: String, CaseIterable, Identifiable {
    case blue, indigo, teal, mint, green, yellow, orange, red, pink, purple, brown, graphite

    public var id: String { rawValue }
    public static let storageKey = "app_accent"

    public var title: String { rawValue.capitalized }

    public var color: Color {
        switch self {
        case .blue: return Color(red: 0.04, green: 0.52, blue: 1.0)
        case .indigo: return Color(red: 0.35, green: 0.34, blue: 0.84)
        case .teal: return Color(red: 0.19, green: 0.69, blue: 0.78)
        case .mint: return Color(red: 0.0, green: 0.78, blue: 0.70)
        case .yellow: return Color(red: 1.0, green: 0.80, blue: 0.0)
        case .brown: return Color(red: 0.64, green: 0.52, blue: 0.37)
        case .green: return Color(red: 0.20, green: 0.78, blue: 0.35)
        case .orange: return Color(red: 1.0, green: 0.58, blue: 0.0)
        case .red: return Color(red: 1.0, green: 0.27, blue: 0.23)
        case .pink: return Color(red: 1.0, green: 0.31, blue: 0.55)
        case .purple: return Color(red: 0.69, green: 0.32, blue: 0.87)
        case .graphite: return Color(red: 0.56, green: 0.56, blue: 0.58)
        }
    }

    /// Color for a stored value: a preset name, or `#rrggbb` picked in the editor.
    public static func color(for raw: String) -> Color {
        if raw.hasPrefix("#"), raw.count == 7 { return Color(hex: String(raw.dropFirst())) }
        return (AppAccent(rawValue: raw) ?? .blue).color
    }

    public static var storedRaw: String {
        UserDefaults.standard.string(forKey: storageKey) ?? ""
    }

    public static var currentColor: Color { color(for: storedRaw) }

    public static var current: AppAccent {
        AppAccent(rawValue: UserDefaults.standard.string(forKey: storageKey) ?? "") ?? .blue
    }
}

extension Color {
    /// The chosen accent. Read when a view is built, so sheets pick up a change when they are
    /// opened again; the main screen observes the setting and rebuilds at once.
    public static var appAccent: Color { AppAccent.currentColor }
}

// MARK: - Home screen icon

/// The home screen icon: a design (classic, light, or one of the grid designs from docs/icons)
/// plus an accent tint. Alternate asset sets are named `AppIcon-<Tint>` (classic),
/// `AppIcon-Light`, `AppIcon-<Design>` (white) and `AppIcon-<Design>-<Tint>`.
public struct AppIconChoice: Equatable {
    public enum Design: String, CaseIterable, Identifiable {
        case classic, gridMinimal = "GridMinimal", gridPrompts = "GridPrompts", gridRounded = "GridRounded"
        case gridSymbols = "GridSymbols", gridSymbolsBold = "GridSymbolsBold", light = "Light"

        public var id: String { rawValue }
        public var title: String {
            switch self {
            case .classic: return "Classic"
            case .gridMinimal: return "Grid minimal"
            case .gridPrompts: return "Grid prompts"
            case .gridRounded: return "Grid rounded"
            case .gridSymbols: return "Grid symbols"
            case .gridSymbolsBold: return "Grid bold"
            case .light: return "Light"
            }
        }
        /// Asset name of the picture shown in Settings for the grid designs.
        public var previewImage: String? { rawValue.hasPrefix("Grid") ? "IconPreview-\(rawValue)" : nil }
        public var supportsTint: Bool { self != .light }
    }

    public enum Tint: String, CaseIterable, Identifiable {
        case white = "", blue = "Blue", teal = "Teal", green = "Green", orange = "Orange"
        case red = "Red", pink = "Pink", purple = "Purple"

        public var id: String { rawValue }
        public var title: String { self == .white ? "White" : rawValue }
        public var color: Color { self == .white ? .white : (AppAccent(rawValue: rawValue.lowercased())?.color ?? .white) }
    }

    public var design: Design
    public var tint: Tint

    public static let standard = AppIconChoice(design: .classic, tint: .white)

    public var alternateName: String? {
        switch design {
        case .light: return "AppIcon-Light"
        case .classic: return tint == .white ? nil : "AppIcon-\(tint.rawValue)"
        default: return tint == .white ? "AppIcon-\(design.rawValue)" : "AppIcon-\(design.rawValue)-\(tint.rawValue)"
        }
    }

    /// Every alternate icon name (for `ASSETCATALOG_COMPILER_ALTERNATE_APPICON_NAMES`).
    public static var allAlternateNames: [String] {
        var names: [String] = []
        for d in Design.allCases {
            for t in Tint.allCases where d.supportsTint || t == .white {
                if let n = AppIconChoice(design: d, tint: t).alternateName { names.append(n) }
            }
        }
        return names
    }

    public static func parse(alternateName: String?) -> AppIconChoice {
        guard let raw = alternateName?.replacingOccurrences(of: "AppIcon-", with: ""), !raw.isEmpty else { return .standard }
        if raw == "Light" { return AppIconChoice(design: .light, tint: .white) }
        if let t = Tint(rawValue: raw), t != .white { return AppIconChoice(design: .classic, tint: t) }
        let parts = raw.split(separator: "-", maxSplits: 1).map(String.init)
        if let d = Design(rawValue: parts[0]) {
            let t = parts.count > 1 ? (Tint(rawValue: parts[1]) ?? .white) : .white
            return AppIconChoice(design: d, tint: t)
        }
        return .standard
    }

    #if canImport(UIKit)
    @MainActor public static var current: AppIconChoice { parse(alternateName: UIApplication.shared.alternateIconName) }

    @MainActor public func apply() async -> String? {
        guard UIApplication.shared.supportsAlternateIcons else { return "This device does not support alternate icons." }
        do {
            try await UIApplication.shared.setAlternateIconName(alternateName)
            return nil
        } catch {
            return error.localizedDescription
        }
    }
    #endif
}

// MARK: - Tab and profile icons

public enum ProfileIcons {
    /// Offered in the profile editor; names that this iOS does not have are skipped.
    public static let symbols: [String] = [
        "terminal", "brain", "sparkles", "cpu", "bolt.fill", "star.fill", "hammer.fill",
        "wrench.and.screwdriver.fill", "briefcase.fill", "house.fill", "person.fill", "flame.fill",
        "leaf.fill", "moon.fill", "sun.max.fill", "globe", "server.rack", "shippingbox.fill",
        "ladybug.fill", "wand.and.stars", "flask.fill", "book.fill", "lock.fill", "heart.fill",
    ].filter { UIImage(systemName: $0) != nil }

    /// Text glyphs from the icon sheet in docs/icons; stored as `glyph:<text>`.
    public static let glyphs: [String] = [">_", "$", "~", "#", "▮", ">", "[]", "—", "%"]
    public static func glyphRef(_ g: String) -> String { "glyph:\(g)" }

    private static var imageDir: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("ProfileIcons", isDirectory: true)
    }

    /// Saves a picture the user picked as a small PNG; returns the stored reference `img:<file>`.
    public static func storeImage(_ data: Data, replacing old: String = "") -> String? {
        guard let image = UIImage(data: data) else { return nil }
        let side: CGFloat = 96
        let size = CGSize(width: side, height: side)
        let fmt = UIGraphicsImageRendererFormat.default()
        fmt.scale = 1
        let scaled = UIGraphicsImageRenderer(size: size, format: fmt).image { _ in
            // Fill the square, centered (crop the longer side).
            let k = max(side / image.size.width, side / image.size.height)
            let w = image.size.width * k, h = image.size.height * k
            image.draw(in: CGRect(x: (side - w) / 2, y: (side - h) / 2, width: w, height: h))
        }
        guard let png = scaled.pngData() else { return nil }
        let name = "\(UUID().uuidString.prefix(12).lowercased()).png"
        do {
            try FileManager.default.createDirectory(at: imageDir, withIntermediateDirectories: true)
            try png.write(to: imageDir.appendingPathComponent(name), options: .atomic)
        } catch { return nil }
        removeImage(old)
        return "img:\(name)"
    }

    public static func removeImage(_ ref: String) {
        guard let file = imageFile(ref) else { return }
        try? FileManager.default.removeItem(at: file)
    }

    private static func imageFile(_ ref: String) -> URL? {
        guard ref.hasPrefix("img:") else { return nil }
        let name = String(ref.dropFirst(4))
        guard !name.isEmpty, !name.contains("/"), !name.contains("..") else { return nil }
        return imageDir.appendingPathComponent(name)
    }

    public static func image(_ ref: String) -> UIImage? {
        guard let file = imageFile(ref) else { return nil }
        return UIImage(contentsOfFile: file.path)
    }

    /// Can this stored reference be drawn (SF Symbol, glyph or an existing picture)?
    public static func isValid(_ ref: String) -> Bool {
        if ref.isEmpty { return false }
        if ref.hasPrefix("glyph:") { return ref.count > 6 && ref.count <= 10 }
        if ref.hasPrefix("img:") { return image(ref) != nil }
        return UIImage(systemName: ref) != nil
    }

    public static func defaultSymbol(for agent: AgentKind) -> String {
        switch agent {
        case .claudeCode: return AgentType.claudeCode.iconName
        case .antigravity: return AgentType.antigravity.iconName
        case .codex: return "chevron.left.forwardslash.chevron.right"
        case .grok: return "bolt.horizontal"
        case .openCode: return "chevron.left.slash.chevron.right"
        case .qwenCode: return "q.circle"
        case .crush: return "sparkles"
        case .cline: return "bubble.left.and.bubble.right"
        case .aider: return "pencil.and.outline"
        case .goose: return "bird"
        case .ollamaChat: return "cube"
        }
    }

    /// The profile's own icon, else the one for the tab's agent.
    public static func symbol(_ profile: SavedAgentProfile?, tabType: AgentType) -> String {
        if let s = profile?.iconSymbol, isValid(s) { return s }
        return tabType.iconName
    }

    /// Icon of a profile in lists (before any tab exists).
    public static func rowSymbol(_ profile: SavedAgentProfile) -> String {
        if isValid(profile.iconSymbol) { return profile.iconSymbol }
        return defaultSymbol(for: profile.agent)
    }

    public static func color(_ profile: SavedAgentProfile?) -> Color? {
        AppAccent(rawValue: profile?.iconColor ?? "")?.color
    }
}

/// Draws a stored icon reference: SF Symbol, text glyph or the user's own picture.
public struct ProfileIconView: View {
    public let ref: String
    public var size: CGFloat = 14

    public init(ref: String, size: CGFloat = 14) {
        self.ref = ref
        self.size = size
    }

    public var body: some View {
        if ref.hasPrefix("glyph:") {
            Text(verbatim: String(ref.dropFirst(6)))
                .font(.system(size: size, weight: .bold, design: .monospaced))
                .lineLimit(1)
        } else if ref.hasPrefix("img:"), let image = ProfileIcons.image(ref) {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
                .frame(width: size * 1.3, height: size * 1.3)
                .clipShape(RoundedRectangle(cornerRadius: size * 0.3))
        } else {
            Image(systemName: ref).font(.system(size: size))
        }
    }
}

// MARK: - Terminal colour schemes

/// Colours of the terminal itself: background, text, cursor and the 16 ANSI colours (hex RGB).
public struct TerminalScheme: Identifiable, Equatable, Codable {
    public var id: String
    public var name: String
    public var background: String
    public var foreground: String
    public var cursor: String
    /// Normal 0-7 then bright 8-15.
    public var ansi: [String]

    public static let builtIn: [TerminalScheme] = [
        TerminalScheme(id: "default", name: "Default", background: "000000", foreground: "e5e5e5", cursor: "ffffff",
            ansi: ["000000", "cd3131", "0dbc79", "e5e510", "2472c8", "bc3fbc", "11a8cd", "e5e5e5",
                   "666666", "f14c4c", "23d18b", "f5f543", "3b8eea", "d670d6", "29b8db", "ffffff"]),
        TerminalScheme(id: "gruvbox", name: "Gruvbox Dark", background: "282828", foreground: "ebdbb2", cursor: "fe8019",
            ansi: ["282828", "cc241d", "98971a", "d79921", "458588", "b16286", "689d6a", "a89984",
                   "928374", "fb4934", "b8bb26", "fabd2f", "83a598", "d3869b", "8ec07c", "ebdbb2"]),
        TerminalScheme(id: "tokyonight", name: "Tokyo Night", background: "1a1b26", foreground: "c0caf5", cursor: "c0caf5",
            ansi: ["15161e", "f7768e", "9ece6a", "e0af68", "7aa2f7", "bb9af7", "7dcfff", "a9b1d6",
                   "414868", "f7768e", "9ece6a", "e0af68", "7aa2f7", "bb9af7", "7dcfff", "c0caf5"]),
        TerminalScheme(id: "nord", name: "Nord", background: "2e3440", foreground: "d8dee9", cursor: "88c0d0",
            ansi: ["3b4252", "bf616a", "a3be8c", "ebcb8b", "81a1c1", "b48ead", "88c0d0", "e5e9f0",
                   "4c566a", "bf616a", "a3be8c", "ebcb8b", "81a1c1", "b48ead", "8fbcbb", "eceff4"]),
        TerminalScheme(id: "dracula", name: "Dracula", background: "282a36", foreground: "f8f8f2", cursor: "ff79c6",
            ansi: ["21222c", "ff5555", "50fa7b", "f1fa8c", "bd93f9", "ff79c6", "8be9fd", "f8f8f2",
                   "6272a4", "ff6e6e", "69ff94", "ffffa5", "d6acff", "ff92df", "a4ffff", "ffffff"]),
        TerminalScheme(id: "solarized", name: "Solarized Dark", background: "002b36", foreground: "839496", cursor: "93a1a1",
            ansi: ["073642", "dc322f", "859900", "b58900", "268bd2", "d33682", "2aa198", "eee8d5",
                   "002b36", "cb4b16", "586e75", "657b83", "839496", "6c71c4", "93a1a1", "fdf6e3"]),
        TerminalScheme(id: "light", name: "Paper (light)", background: "fafafa", foreground: "383a42", cursor: "526fff",
            ansi: ["383a42", "e45649", "50a14f", "c18401", "4078f2", "a626a4", "0184bc", "a0a1a4",
                   "4f525e", "e06c75", "98c379", "e5c07b", "61afef", "c678dd", "56b6c2", "ffffff"]),
        TerminalScheme(id: "catppuccin", name: "Catppuccin Mocha", background: "1e1e2e", foreground: "cdd6f4", cursor: "f5e0dc",
            ansi: ["45475a", "f38ba8", "a6e3a1", "f9e2af", "89b4fa", "f5c2e7", "94e2d5", "bac2de",
                   "585b70", "f38ba8", "a6e3a1", "f9e2af", "89b4fa", "f5c2e7", "94e2d5", "a6adc8"]),
        TerminalScheme(id: "monokai", name: "Monokai", background: "272822", foreground: "f8f8f2", cursor: "f8f8f0",
            ansi: ["272822", "f92672", "a6e22e", "f4bf75", "66d9ef", "ae81ff", "a1efe4", "f8f8f2",
                   "75715e", "f92672", "a6e22e", "f4bf75", "66d9ef", "ae81ff", "a1efe4", "f9f8f5"]),
        TerminalScheme(id: "onedark", name: "One Dark", background: "282c34", foreground: "abb2bf", cursor: "528bff",
            ansi: ["282c34", "e06c75", "98c379", "e5c07b", "61afef", "c678dd", "56b6c2", "abb2bf",
                   "545862", "e06c75", "98c379", "e5c07b", "61afef", "c678dd", "56b6c2", "c8ccd4"]),
        TerminalScheme(id: "rosepine", name: "Rosé Pine", background: "191724", foreground: "e0def4", cursor: "e0def4",
            ansi: ["26233a", "eb6f92", "31748f", "f6c177", "9ccfd8", "c4a7e7", "ebbcba", "e0def4",
                   "6e6a86", "eb6f92", "31748f", "f6c177", "9ccfd8", "c4a7e7", "ebbcba", "e0def4"]),
        TerminalScheme(id: "everforest", name: "Everforest", background: "2d353b", foreground: "d3c6aa", cursor: "d3c6aa",
            ansi: ["475258", "e67e80", "a7c080", "dbbc7f", "7fbbb3", "d699b6", "83c092", "d3c6aa",
                   "475258", "e67e80", "a7c080", "dbbc7f", "7fbbb3", "d699b6", "83c092", "d3c6aa"]),
        TerminalScheme(id: "githubdark", name: "GitHub Dark", background: "0d1117", foreground: "c9d1d9", cursor: "58a6ff",
            ansi: ["484f58", "ff7b72", "3fb950", "d29922", "58a6ff", "bc8cff", "39c5cf", "b1bac4",
                   "6e7681", "ffa198", "56d364", "e3b341", "79c0ff", "d2a8ff", "56d4dd", "f0f6fc"]),
        TerminalScheme(id: "highcontrast", name: "High contrast", background: "000000", foreground: "ffffff", cursor: "ffffff",
            ansi: ["000000", "ff5555", "55ff55", "ffff55", "5599ff", "ff55ff", "55ffff", "ffffff",
                   "7f7f7f", "ff8080", "80ff80", "ffff80", "80b3ff", "ff80ff", "80ffff", "ffffff"]),
        TerminalScheme(id: "retrogreen", name: "Retro green", background: "000000", foreground: "33ff33", cursor: "33ff33",
            ansi: ["000000", "ff3333", "33ff33", "cccc33", "3366ff", "cc33cc", "33cccc", "33ff33",
                   "1a801a", "ff6666", "66ff66", "ffff66", "6699ff", "ff66ff", "66ffff", "99ff99"]),
        TerminalScheme(id: "solarizedlight", name: "Solarized Light", background: "fdf6e3", foreground: "657b83", cursor: "586e75",
            ansi: ["073642", "dc322f", "859900", "b58900", "268bd2", "d33682", "2aa198", "eee8d5",
                   "002b36", "cb4b16", "586e75", "657b83", "839496", "6c71c4", "93a1a1", "fdf6e3"]),
        TerminalScheme(id: "latte", name: "Catppuccin Latte", background: "eff1f5", foreground: "4c4f69", cursor: "dc8a78",
            ansi: ["5c5f77", "d20f39", "40a02b", "df8e1d", "1e66f5", "ea76cb", "179299", "acb0be",
                   "6c6f85", "d20f39", "40a02b", "df8e1d", "1e66f5", "ea76cb", "179299", "bcc0cc"]),
    ]

    public init(id: String, name: String, background: String, foreground: String, cursor: String, ansi: [String]) {
        self.id = id
        self.name = name
        self.background = background
        self.foreground = foreground
        self.cursor = cursor
        self.ansi = ansi
    }

    public var isCustom: Bool { id.hasPrefix("custom-") }

    private static let customKey = "terminal_custom_schemes"

    /// Schemes the user made in the editor (stored on the phone).
    public static var custom: [TerminalScheme] {
        get {
            guard let data = UserDefaults.standard.data(forKey: customKey),
                  let list = try? JSONDecoder().decode([TerminalScheme].self, from: data) else { return [] }
            return list.filter { $0.ansi.count == 16 }
        }
        set {
            if let data = try? JSONEncoder().encode(newValue) { UserDefaults.standard.set(data, forKey: customKey) }
        }
    }

    public static var all: [TerminalScheme] { builtIn + custom }

    public static func upsertCustom(_ scheme: TerminalScheme) {
        var list = custom
        if let i = list.firstIndex(where: { $0.id == scheme.id }) { list[i] = scheme } else { list.append(scheme) }
        custom = list
    }

    public static func deleteCustom(id: String) { custom = custom.filter { $0.id != id } }

    /// A fresh custom scheme starting from `base`.
    public static func newCustom(from base: TerminalScheme, name: String) -> TerminalScheme {
        TerminalScheme(id: "custom-\(UUID().uuidString.prefix(8).lowercased())", name: name,
                       background: base.background, foreground: base.foreground, cursor: base.cursor, ansi: base.ansi)
    }

    public static func scheme(id: String) -> TerminalScheme? { all.first { $0.id == id } }
    public static let fallback = builtIn[0]
}

/// Which agent a tab runs, for choosing its default colours.
public enum SchemeSlot: String, CaseIterable, Identifiable {
    case general, claude, antigravity, codex, grok, local, shell

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .general: return "Default (anything else)"
        case .claude: return "Claude Code"
        case .antigravity: return "Antigravity"
        case .codex: return "Codex"
        case .grok: return "Grok"
        case .local: return "Local-model agents"
        case .shell: return "Plain shell"
        }
    }

    /// Built-in assignment until the user picks one: each agent looks different at a glance.
    var builtInChoice: String {
        switch self {
        case .general, .shell: return "default"
        case .claude: return "gruvbox"
        case .antigravity: return "tokyonight"
        case .codex: return "nord"
        case .grok: return "dracula"
        case .local: return "solarized"
        }
    }

    var storageKey: String { "terminal_scheme_\(rawValue)" }

    public static func forTab(_ type: AgentType) -> SchemeSlot {
        switch type {
        case .claudeCode: return .claude
        case .antigravity: return .antigravity
        case .localLlm, .customShell: return .shell
        }
    }

    public static func forAgent(_ kind: AgentKind) -> SchemeSlot {
        switch kind {
        case .claudeCode: return .claude
        case .antigravity: return .antigravity
        case .codex: return .codex
        case .grok: return .grok
        case .openCode, .qwenCode, .crush, .cline, .aider, .goose, .ollamaChat: return .local
        }
    }
}

public enum TerminalSchemes {
    /// The scheme id chosen for a slot (or its built-in choice).
    public static func choice(for slot: SchemeSlot) -> String {
        let saved = UserDefaults.standard.string(forKey: slot.storageKey) ?? ""
        return TerminalScheme.scheme(id: saved) != nil ? saved : slot.builtInChoice
    }

    public static func setChoice(_ id: String, for slot: SchemeSlot) {
        UserDefaults.standard.set(id, forKey: slot.storageKey)
    }

    /// Profile override first, then the slot of its agent (or the tab's), then the default.
    public static func resolve(tabSchemeId: String? = nil, profileSchemeId: String?, slot: SchemeSlot) -> TerminalScheme {
        if let id = tabSchemeId, let s = TerminalScheme.scheme(id: id) { return s }
        if let id = profileSchemeId, let s = TerminalScheme.scheme(id: id) { return s }
        return TerminalScheme.scheme(id: choice(for: slot)) ?? .fallback
    }
}

extension Color {
    /// `rrggbb` of this color (sRGB).
    var hexString: String {
        #if canImport(UIKit)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(self).getRed(&r, green: &g, blue: &b, alpha: &a)
        return String(format: "%02x%02x%02x", Int((r * 255).rounded()), Int((g * 255).rounded()), Int((b * 255).rounded()))
        #else
        return "000000"
        #endif
    }

    init(hex: String) {
        let v = UInt32(hex, radix: 16) ?? 0
        self.init(red: Double((v >> 16) & 0xff) / 255, green: Double((v >> 8) & 0xff) / 255, blue: Double(v & 0xff) / 255)
    }
}

#if canImport(UIKit)
extension UIColor {
    convenience init(hex: String) {
        let v = UInt32(hex, radix: 16) ?? 0
        self.init(red: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255, blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }
}
#endif
