import Foundation
import Highlightr
#if canImport(UIKit)
import UIKit
#endif

/// highlight.js through the Highlightr package (about 190 languages, runs in JavaScriptCore).
/// Highlightr is not thread-safe, so everything goes through this actor.
actor HighlightrEngine {
    private var engine: Highlightr?
    private var themeName: String?
    private var languages: Set<String>?

    private func ready() -> Highlightr? {
        if engine == nil { engine = Highlightr() }
        return engine
    }

    func supportedLanguages() -> Set<String> {
        if let languages { return languages }
        let all = Set(ready()?.supportedLanguages() ?? [])
        languages = all
        return all
    }

    func highlight(_ code: String, language: String, themeName: String) -> [HighlightedLine]? {
        guard let engine = ready(), supportedLanguages().contains(language) else { return nil }
        if self.themeName != themeName {
            guard engine.setTheme(to: themeName) else { return nil }
            self.themeName = themeName
        }
        guard let attributed = engine.highlight(code, as: language, fastRender: true) else { return nil }
        return Self.lines(from: attributed)
    }

    /// Splits the attributed text into lines of coloured runs; only the colours are kept, the
    /// view chooses font and size.
    private static func lines(from attributed: NSAttributedString) -> [HighlightedLine] {
        let text = attributed.string as NSString
        var lines: [HighlightedLine] = []
        var current: [SyntaxRun] = []
        attributed.enumerateAttribute(
            .foregroundColor, in: NSRange(location: 0, length: attributed.length)
        ) { value, range, _ in
            var color: SyntaxColor?
            #if canImport(UIKit)
            if let ui = value as? UIColor {
                var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
                if ui.getRed(&r, green: &g, blue: &b, alpha: &a) {
                    color = SyntaxColor(red: Double(r), green: Double(g), blue: Double(b))
                }
            }
            #endif
            let parts = text.substring(with: range).components(separatedBy: "\n")
            for (i, part) in parts.enumerated() {
                if i > 0 {
                    lines.append(HighlightedLine(runs: current))
                    current = []
                }
                let clean = part.replacingOccurrences(of: "\r", with: "")
                if !clean.isEmpty { current.append(SyntaxRun(text: clean, color: color)) }
            }
        }
        lines.append(HighlightedLine(runs: current))
        return lines
    }
}

struct HighlightrModule: SyntaxModule {
    static let shared = HighlightrModule()
    private let engine = HighlightrEngine()

    let id = "highlightjs"
    let title = "highlight.js"
    let summary = "Broad coverage: about 190 languages and file types, from Rust and Swift to Dockerfile, nginx and SQL."

    func supports(language: String) async -> Bool {
        await engine.supportedLanguages().contains(language)
    }

    func languageCount() async -> Int { await engine.supportedLanguages().count }

    func highlight(_ code: String, language: String, themeName: String) async -> [HighlightedLine]? {
        await engine.highlight(code, language: language, themeName: themeName)
    }
}
