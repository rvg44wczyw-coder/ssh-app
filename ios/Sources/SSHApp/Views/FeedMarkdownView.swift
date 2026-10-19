import SwiftUI

/// Splitting and inline parsing of an agent's message. Fenced code blocks are separated from the
/// prose; the rest is inline Markdown. Nothing is rendered as HTML or images, and links are never
/// opened automatically.
enum FeedMarkdown {
    enum Segment: Equatable {
        case prose(String)
        case code(language: String, code: String)
    }

    /// Splits on ``` fences; an unclosed fence (streaming) runs to the end.
    static func segments(_ text: String) -> [Segment] {
        var out: [Segment] = []
        var prose: [Substring] = []
        var code: [Substring] = []
        var lang = ""
        var inCode = false
        func flushProse() {
            let joined = prose.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !joined.isEmpty { out.append(.prose(joined)) }
            prose = []
        }
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if inCode {
                    out.append(.code(language: lang, code: code.joined(separator: "\n")))
                    code = []
                    inCode = false
                } else {
                    flushProse()
                    lang = String(t.dropFirst(3)).trimmingCharacters(in: .whitespaces).lowercased()
                    inCode = true
                }
            } else if inCode {
                code.append(line)
            } else {
                prose.append(line)
            }
        }
        if inCode { out.append(.code(language: lang, code: code.joined(separator: "\n"))) }
        flushProse()
        return out
    }

    static func inline(_ s: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: s, options: options)) ?? AttributedString(s)
    }
}
