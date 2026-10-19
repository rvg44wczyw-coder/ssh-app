import SwiftUI

/// An agent's message as readable text: fenced code blocks in their own selectable box (with a
/// copy button and syntax colours when enabled), the rest as inline Markdown. Everything is
/// selectable. Links are not opened automatically; HTML and images are not rendered.
struct FeedMarkdownView: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.segments(text).enumerated()), id: \.offset) { _, seg in
                switch seg {
                case .prose(let p):
                    Text(Self.inline(p))
                        .font(.system(size: 14))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                case .code(let lang, let code):
                    FeedCodeBlock(language: lang, code: code)
                }
            }
        }
    }

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

private struct FeedCodeBlock: View {
    let language: String
    let code: String
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage(SyntaxSettings.enabledKey) private var highlightEnabled = true
    @State private var lines: [HighlightedLine]?
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language.isEmpty ? "code" : language)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundColor(.secondary)
                Spacer()
                Button {
                    #if canImport(UIKit)
                    UIPasteboard.general.string = code
                    #endif
                    copied = true
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 11))
                }
                .buttonStyle(.plain)
                .foregroundColor(.secondary)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            ScrollView(.horizontal, showsIndicators: false) {
                content
                    .font(.system(size: 12, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(8)
            }
        }
        .background(Color.black.opacity(0.28))
        .cornerRadius(8)
        .task(id: "\(highlightEnabled)|\(colorScheme == .dark)|\(code.count)") {
            guard highlightEnabled, !language.isEmpty, code.utf8.count < 20_000 else {
                lines = nil
                return
            }
            lines = await SyntaxService.shared.highlight(
                text: code, filename: "code." + Self.ext(language), dark: colorScheme == .dark)
        }
    }

    @ViewBuilder private var content: some View {
        if let lines {
            Text(lines.reduce(into: AttributedString()) { acc, line in
                if !acc.characters.isEmpty { acc.append(AttributedString("\n")) }
                acc.append(line.attributed())
            })
        } else {
            Text(code)
        }
    }

    private static func ext(_ lang: String) -> String {
        let map = [
            "python": "py", "javascript": "js", "typescript": "ts", "rust": "rs", "bash": "sh",
            "shell": "sh", "zsh": "sh", "yaml": "yml", "markdown": "md", "golang": "go",
            "kotlin": "kt", "objective-c": "m", "c++": "cpp", "csharp": "cs",
        ]
        return map[lang] ?? lang
    }
}
