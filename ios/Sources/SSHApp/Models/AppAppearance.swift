import SwiftUI
import SshCoreBridge

#if canImport(UIKit)
import UIKit
#endif

// MARK: - Accent colour

/// The app's accent colour (buttons, selected chips, links). Stored as `app_accent`.
public enum AppAccent: String, CaseIterable, Identifiable {
    case blue, teal, green, orange, red, pink, purple, graphite

    public var id: String { rawValue }
    public static let storageKey = "app_accent"

    public var title: String { rawValue.capitalized }

    public var color: Color {
        switch self {
        case .blue: return Color(red: 0.04, green: 0.52, blue: 1.0)
        case .teal: return Color(red: 0.19, green: 0.69, blue: 0.78)
        case .green: return Color(red: 0.20, green: 0.78, blue: 0.35)
        case .orange: return Color(red: 1.0, green: 0.58, blue: 0.0)
        case .red: return Color(red: 1.0, green: 0.27, blue: 0.23)
        case .pink: return Color(red: 1.0, green: 0.31, blue: 0.55)
        case .purple: return Color(red: 0.69, green: 0.32, blue: 0.87)
        case .graphite: return Color(red: 0.56, green: 0.56, blue: 0.58)
        }
    }

    public static var current: AppAccent {
        AppAccent(rawValue: UserDefaults.standard.string(forKey: storageKey) ?? "") ?? .blue
    }
}

extension Color {
    /// The chosen accent. Read when a view is built, so sheets pick up a change when they are
    /// opened again; the main screen observes the setting and rebuilds at once.
    public static var appAccent: Color { AppAccent.current.color }
}

// MARK: - Terminal colour schemes

/// Colours of the terminal itself: background, text, cursor and the 16 ANSI colours (hex RGB).
public struct TerminalScheme: Identifiable, Equatable {
    public let id: String
    public let name: String
    public let background: String
    public let foreground: String
    public let cursor: String
    /// Normal 0-7 then bright 8-15.
    public let ansi: [String]

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
    ]

    public static func scheme(id: String) -> TerminalScheme? { builtIn.first { $0.id == id } }
    public static let fallback = builtIn[0]
}

/// Which agent a tab runs, for choosing its default colours.
public enum SchemeSlot: String, CaseIterable, Identifiable {
    case general, claude, antigravity, codex, grok, shell

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .general: return "Default (anything else)"
        case .claude: return "Claude Code"
        case .antigravity: return "Antigravity"
        case .codex: return "Codex"
        case .grok: return "Grok"
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
    public static func resolve(profileSchemeId: String?, slot: SchemeSlot) -> TerminalScheme {
        if let id = profileSchemeId, let s = TerminalScheme.scheme(id: id) { return s }
        return TerminalScheme.scheme(id: choice(for: slot)) ?? .fallback
    }
}

extension Color {
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
