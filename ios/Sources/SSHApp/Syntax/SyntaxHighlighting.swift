import Foundation

/// A coloured piece of a line. `color` is nil for the default text colour.
struct SyntaxRun: Sendable, Equatable {
    var text: String
    var color: SyntaxColor?
}

struct SyntaxColor: Sendable, Equatable {
    var red: Double
    var green: Double
    var blue: Double
}

struct HighlightedLine: Sendable, Equatable {
    var runs: [SyntaxRun]
}

/// Colour themes, as families with a dark and a light variant so the code stays readable in
/// whichever appearance the app is in.
enum SyntaxTheme: String, CaseIterable, Identifiable, Sendable {
    case auto, atomOne, github, solarized, monokai, vs, xcode, gruvbox, tokyoNight, stackoverflow

    var id: String { rawValue }

    var title: String {
        switch self {
        case .auto: return "Automatic"
        case .atomOne: return "Atom One"
        case .github: return "GitHub"
        case .solarized: return "Solarized"
        case .monokai: return "Monokai"
        case .vs: return "Visual Studio"
        case .xcode: return "Xcode"
        case .gruvbox: return "Gruvbox"
        case .tokyoNight: return "Tokyo Night"
        case .stackoverflow: return "Stack Overflow"
        }
    }

    /// Theme file names of the highlight.js stylesheet set, for the dark and the light variant.
    func names(dark: Bool) -> String {
        switch self {
        case .auto, .atomOne: return dark ? "atom-one-dark" : "atom-one-light"
        case .github: return dark ? "github-dark" : "github"
        case .solarized: return dark ? "solarized-dark" : "solarized-light"
        case .monokai: return dark ? "monokai-sublime" : "xcode"
        case .vs: return dark ? "vs2015" : "vs"
        case .xcode: return dark ? "xcode-dark" : "xcode"
        case .gruvbox: return dark ? "gruvbox-dark-medium" : "gruvbox-light-medium"
        case .tokyoNight: return dark ? "tokyo-night-dark" : "tokyo-night-light"
        case .stackoverflow: return dark ? "stackoverflow-dark" : "stackoverflow-light"
        }
    }
}

/// One highlighting engine. The app asks every enabled module in turn; the first that knows the
/// language renders it. A module never sees anything but the text, a language id and a theme.
protocol SyntaxModule: Sendable {
    var id: String { get }
    var title: String { get }
    var summary: String { get }
    func supports(language: String) async -> Bool
    func languageCount() async -> Int
    func highlight(_ code: String, language: String, themeName: String) async -> [HighlightedLine]?
}

/// User choices, stored in UserDefaults (so Settings can bind them with `@AppStorage`).
enum SyntaxSettings {
    static let enabledKey = "syntax_highlight_enabled"
    static let themeKey = "syntax_highlight_theme"
    static let maxKBKey = "syntax_highlight_max_kb"
    static func moduleKey(_ id: String) -> String { "syntax_module_\(id)" }

    static var enabled: Bool {
        UserDefaults.standard.object(forKey: enabledKey) as? Bool ?? true
    }

    static var theme: SyntaxTheme {
        SyntaxTheme(rawValue: UserDefaults.standard.string(forKey: themeKey) ?? "") ?? .auto
    }

    /// Larger files are shown plain: highlighting runs on the phone and a huge file would stall.
    static var maxBytes: Int {
        let kb = UserDefaults.standard.object(forKey: maxKBKey) as? Int ?? 200
        return min(max(kb, 10), 2000) * 1024
    }

    static func isModuleEnabled(_ id: String) -> Bool {
        UserDefaults.standard.object(forKey: moduleKey(id)) as? Bool ?? true
    }
}

/// Finds the module, runs it off the main thread (the engines are not thread-safe, so one request
/// at a time) and remembers the last result.
actor SyntaxService {
    static let shared = SyntaxService()

    /// The available engines. A new one (for example a Rust `syntect` module) is added here.
    static let modules: [any SyntaxModule] = [HighlightrModule.shared]

    private var lastKey: Int?
    private var lastResult: [HighlightedLine]?

    func highlight(text: String, filename: String, dark: Bool) async -> [HighlightedLine]? {
        guard SyntaxSettings.enabled, text.utf8.count <= SyntaxSettings.maxBytes else { return nil }
        let firstLine = text.prefix(while: { $0 != "\n" })
        guard let language = SyntaxLanguages.id(forFilename: filename, firstLine: String(firstLine))
        else { return nil }
        let themeName = SyntaxSettings.theme.names(dark: dark)
        var hasher = Hasher()
        hasher.combine(text)
        hasher.combine(language)
        hasher.combine(themeName)
        let key = hasher.finalize()
        if key == lastKey { return lastResult }
        for module in Self.modules where SyntaxSettings.isModuleEnabled(module.id) {
            guard await module.supports(language: language) else { continue }
            let result = await module.highlight(text, language: language, themeName: themeName)
            lastKey = key
            lastResult = result
            return result
        }
        return nil
    }
}
