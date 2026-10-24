import Foundation
import SshCoreBridge

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


/// Finding the real file behind a mention. The agent writes the same file in different ways: a full
/// path, a path from the project folder, a path from where it stands, or a bare name. The app does
/// not rely on how it was written: it looks at the files the session has touched (absolute paths in
/// the agent's own tool calls), then at the session's folders, then asks the host to search the project.
enum FileRefResolver {
    private static let pathKeys = ["file_path", "path", "notebook_path", "filePath", "filepath"]

    /// Absolute paths the agent has read, edited or written in this session, most recent first.
    static func touchedPaths(in turns: [TranscriptTurn], limit: Int = 500) -> [String] {
        var seen = Set<String>()
        var out: [String] = []
        for turn in turns.reversed() {
            for call in turn.toolCalls.reversed() {
                guard let json = call.fullArgumentsJson, let data = json.data(using: .utf8),
                      let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
                for key in pathKeys {
                    if let path = obj[key] as? String, path.hasPrefix("/"), path.count <= 1000, seen.insert(path).inserted {
                        out.append(path)
                        if out.count >= limit { return out }
                    }
                }
            }
        }
        return out
    }

    private static func components(_ path: String) -> [Substring] {
        path.split(separator: "/").filter { !$0.isEmpty && $0 != "." }
    }

    /// The paths in `known` that end with the mention's components (`Views/Feed.swift` matches
    /// `/p/ios/Views/Feed.swift`; a bare `Feed.swift` matches any folder), in the order of `known`.
    static func matches(_ ref: String, in known: [String]) -> [String] {
        let want = components(ref)
        guard !want.isEmpty else { return [] }
        var seen = Set<String>()
        return known.filter { path in
            let have = components(path)
            return have.count >= want.count && Array(have.suffix(want.count)) == want && seen.insert(path).inserted
        }
    }

    /// A mention is only handed to the host's search when it is a plain relative path.
    static func isSearchable(_ ref: String) -> Bool {
        guard !ref.isEmpty, ref.count <= 200, !ref.hasPrefix("-"), !ref.hasPrefix("/"), !ref.hasPrefix("~"),
              !components(ref).contains(".."),
              ref.range(of: #"^[A-Za-z0-9_.@+/-]+$"#, options: .regularExpression) != nil else { return false }
        return true
    }

    private static func quoted(_ s: String) -> String { "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'" }

    /// The command that lists files named like the mention under `root`: the tracked files of a git
    /// repository (fast, no build folders), else a bounded `find`. Nil for a mention that is not safe.
    static func searchCommand(root: String, ref: String) -> String? {
        guard isSearchable(ref), root.hasPrefix("/"), let name = components(ref).last.map(String.init) else { return nil }
        let n = quoted(name)
        let skip = [".git", "node_modules", "target", "build", ".build", "DerivedData", "Pods", "dist", ".venv"]
            .map { "-name \(quoted($0))" }.joined(separator: " -o ")
        return "cd \(quoted(root)) 2>/dev/null || exit 0; "
            + "out=$(git ls-files -- \(quoted(":(glob)**/" + name)) \(n) 2>/dev/null | head -50); "
            + "if [ -n \"$out\" ]; then printf '%s\\n' \"$out\"; "
            + "else find . -maxdepth 8 \\( \(skip) \\) -prune -o -name \(n) -type f -print 2>/dev/null | head -50; fi"
    }

    /// Absolute paths from the search output (`root`-relative lines), those that really end with the mention.
    static func parseSearch(_ output: String, root: String, ref: String) -> [String] {
        let base = root.hasSuffix("/") ? String(root.dropLast()) : root
        let paths = output.split(separator: "\n").map { line -> String in
            var l = String(line)
            if l.hasPrefix("./") { l.removeFirst(2) }
            return base + "/" + l
        }
        // Closest to the project root first: fewer folders, then alphabetical.
        let ranked = paths.sorted { a, b in
            let ca = components(a).count, cb = components(b).count
            return ca == cb ? a < b : ca < cb
        }
        return matches(ref, in: ranked)
    }
}
