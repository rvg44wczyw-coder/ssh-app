import Foundation

/// A mention of a file in the feed (`src/main.rs`, `Cargo.toml:12`, `lib.rs#L40`): found in inline
/// `code` and in plain text, drawn as a link that opens the file at that line.
struct FeedFileRef: Equatable {
    let path: String
    let line: Int?

    /// Extensions that make a word a file name; anything else (`e.g.`, `v1.2`, `foo.bar`) stays text.
    static let extensions: Set<String> = [
        "rs", "swift", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "json", "toml", "yaml", "yml", "md",
        "txt", "sh", "zsh", "bash", "go", "c", "h", "cpp", "hpp", "cc", "m", "mm", "java", "kt", "kts", "rb",
        "php", "html", "css", "scss", "sql", "xml", "plist", "lock", "conf", "cfg", "ini", "env", "log",
        "gradle", "pbxproj", "entitlements", "strings", "xcconfig", "proto", "lua", "ex", "exs", "cs",
        "dart", "vue", "svelte", "tf", "csv", "puml", "kdl", "swiftpackage", "podspec", "dockerfile",
    ]

    private static let pattern =
        #"(?<![\w/.~@:-])(~?/?(?:[\w.@+-]+/)*[\w.@+-]+\.([A-Za-z][A-Za-z0-9]{0,11}))(?::(\d+)(?::\d+)?|#L(\d+)(?:-L?\d+)?)?(?![\w])"#
    private static let regex = try? NSRegularExpression(pattern: pattern)

    /// Attribute carrying `path` and the line (`path\u{0}line`) on the characters of a mention.
    static let key = NSAttributedString.Key("feed.fileRef")

    var encoded: String { path + "\u{0}" + (line.map(String.init) ?? "") }

    init(path: String, line: Int?) {
        self.path = path
        self.line = line
    }

    init?(encoded: String) {
        let parts = encoded.components(separatedBy: "\u{0}")
        guard parts.count == 2, !parts[0].isEmpty else { return nil }
        self.init(path: parts[0], line: Int(parts[1]))
    }

    private static func ref(_ m: NSTextCheckingResult, in s: NSString) -> FeedFileRef? {
        let path = s.substring(with: m.range(at: 1))
        let ext = s.substring(with: m.range(at: 2)).lowercased()
        guard extensions.contains(ext), !path.hasSuffix("/") else { return nil }
        var line: Int?
        for g in [3, 4] where m.range(at: g).location != NSNotFound {
            line = Int(s.substring(with: m.range(at: g)))
        }
        if let l = line, l < 1 { line = nil }
        return FeedFileRef(path: path, line: line)
    }

    /// The whole text is one file mention (an inline `code` span).
    static func parse(_ text: String) -> FeedFileRef? {
        let s = text.trimmingCharacters(in: .whitespaces) as NSString
        guard s.length > 0, s.length <= 400, let regex,
              let m = regex.firstMatch(in: s as String, range: NSRange(location: 0, length: s.length)),
              m.range.location == 0, m.range.length == s.length else { return nil }
        return ref(m, in: s)
    }

    /// Every file mention in a plain piece of text, with its range.
    static func scan(_ text: String) -> [(range: NSRange, ref: FeedFileRef)] {
        let s = text as NSString
        guard s.length > 2, let regex else { return [] }
        return regex.matches(in: text, range: NSRange(location: 0, length: s.length)).compactMap { m in
            ref(m, in: s).map { (m.range, $0) }
        }
    }
}
