import SwiftUI
import SshCoreBridge

#if canImport(UIKit)
import UIKit

/// The whole feed as ONE native text view, so selection works the way it does in a reader: long
/// press, drag the handles across turns, magnifier, system Copy / Share / Look Up. Headers of
/// "Thinking" and tool calls are links that expand and collapse their body.
struct FeedTextView: UIViewRepresentable {
    let turns: [TranscriptTurn]
    let expandedThinking: Set<String>
    let expandedTools: Set<String>
    let onToggle: (FeedToggle) -> Void
    @Binding var atBottom: Bool
    /// Changes when the user asks to jump to the newest text.
    let scrollToken: Int
    /// Changes when the user asks for the system find bar.
    let findToken: Int
    let dark: Bool
    /// Bumped when syntax colours for code blocks arrived, so the text is rebuilt.
    let highlightVersion: Int

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextView {
        let tv = UITextView()
        tv.isEditable = false
        tv.isSelectable = true
        tv.backgroundColor = .clear
        tv.alwaysBounceVertical = true
        tv.keyboardDismissMode = .interactive
        tv.textContainerInset = UIEdgeInsets(top: 12, left: 12, bottom: 16, right: 12)
        tv.textContainer.lineFragmentPadding = 0
        tv.adjustsFontForContentSizeCategory = true
        tv.linkTextAttributes = [.foregroundColor: UIColor.secondaryLabel]
        tv.isFindInteractionEnabled = true
        tv.delegate = context.coordinator
        return tv
    }

    func updateUIView(_ tv: UITextView, context: Context) {
        let c = context.coordinator
        c.parent = self
        let text = FeedAttributedBuilder.build(
            turns: turns, expandedThinking: expandedThinking, expandedTools: expandedTools, dark: dark)
        if findToken != c.lastFind {
            c.lastFind = findToken
            tv.findInteraction?.presentFindNavigator(showingReplace: false)
        }
        var jump = false
        if scrollToken != c.lastToken {
            c.lastToken = scrollToken
            jump = true
        }
        if !text.isEqual(to: c.lastText) {
            let follow = c.isAtBottom(tv) || c.lastText.length == 0
            let offset = tv.contentOffset
            let selection = tv.selectedRange
            c.lastText = text
            tv.attributedText = text
            if selection.location + selection.length <= text.length { tv.selectedRange = selection }
            tv.layoutIfNeeded()
            if follow || jump {
                c.scrollToEnd(tv)
            } else {
                tv.setContentOffset(offset, animated: false)
            }
        } else if jump {
            c.scrollToEnd(tv)
        }
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: FeedTextView
        var lastText = NSAttributedString()
        var lastToken = 0
        var lastFind = 0
        init(_ parent: FeedTextView) { self.parent = parent }

        func isAtBottom(_ tv: UIScrollView) -> Bool {
            tv.contentOffset.y >= tv.contentSize.height - tv.bounds.height - 40
        }

        func scrollToEnd(_ tv: UITextView) {
            let y = max(-tv.adjustedContentInset.top,
                        tv.contentSize.height - tv.bounds.height + tv.adjustedContentInset.bottom)
            tv.setContentOffset(CGPoint(x: 0, y: y), animated: false)
            report(tv)
        }

        func report(_ tv: UIScrollView) {
            let now = isAtBottom(tv)
            if now != parent.atBottom {
                DispatchQueue.main.async { self.parent.atBottom = now }
            }
        }

        func scrollViewDidScroll(_ scrollView: UIScrollView) { report(scrollView) }

        /// Taps on the feed's own toggle links expand or collapse a turn; no link is ever opened.
        func textView(_ textView: UITextView, primaryActionFor textItem: UITextItem,
                      defaultAction: UIAction) -> UIAction? {
            if case .link(let url) = textItem.content, let toggle = FeedToggle(url: url) {
                return UIAction { [parent] _ in parent.onToggle(toggle) }
            }
            return UIAction { _ in }
        }
    }
}

enum FeedToggle: Equatable {
    case thinking(String)
    case tool(String)

    var url: URL? {
        var comps = URLComponents()
        comps.scheme = "feed"
        switch self {
        case .thinking(let id): comps.host = "thinking"; comps.queryItems = [URLQueryItem(name: "id", value: id)]
        case .tool(let id): comps.host = "tool"; comps.queryItems = [URLQueryItem(name: "id", value: id)]
        }
        return comps.url
    }

    init?(url: URL) {
        guard url.scheme == "feed",
              let comps = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let id = comps.queryItems?.first(where: { $0.name == "id" })?.value
        else { return nil }
        switch url.host {
        case "thinking": self = .thinking(id)
        case "tool": self = .tool(id)
        default: return nil
        }
    }
}

/// Syntax colours for fenced code, filled asynchronously and read synchronously by the builder.
@MainActor
enum FeedHighlightStore {
    private static var cache: [String: [HighlightedLine]] = [:]

    private static func key(_ lang: String, _ code: String, _ dark: Bool) -> String {
        "\(dark ? "d" : "l")|\(lang)|\(code.hashValue)"
    }

    static func lookup(_ lang: String, _ code: String, dark: Bool) -> [HighlightedLine]? {
        cache[key(lang, code, dark)]
    }

    /// Highlights every fenced block of `turns` that is not cached yet; true when something new
    /// was added (the caller rebuilds the text).
    static func prefetch(_ turns: [TranscriptTurn], dark: Bool) async -> Bool {
        guard SyntaxSettings.enabled else { return false }
        var added = false
        for turn in turns {
            guard let content = turn.content else { continue }
            for case .code(let lang, let code) in FeedMarkdown.segments(content) {
                let k = key(lang, code, dark)
                guard !lang.isEmpty, cache[k] == nil, code.utf8.count < 20_000 else { continue }
                if let lines = await SyntaxService.shared.highlight(
                    text: code, filename: "code." + ext(lang), dark: dark) {
                    cache[k] = lines
                    added = true
                }
            }
        }
        if cache.count > 400 { cache.removeAll() }
        return added
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

/// Builds the feed's attributed text in the look of Claude Code's terminal: monospace, `>` for
/// what you wrote, `●` for the agent's text and tool calls, `⎿` for results (long results fold
/// to a few lines). Pure: same turns and expansion state, same text.
@MainActor
enum FeedAttributedBuilder {
    nonisolated private static let size: CGFloat = 13.5
    private static let previewLines = 4

    private static func mono(_ weight: UIFont.Weight = .regular, size: CGFloat = size) -> UIFont {
        UIFont.monospacedSystemFont(ofSize: size, weight: weight)
    }

    private static func style(indent: CGFloat = 0, first: CGFloat? = nil, after: CGFloat = 2) -> NSMutableParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineSpacing = 2
        p.paragraphSpacing = after
        p.firstLineHeadIndent = first ?? indent
        p.headIndent = indent
        return p
    }

    private static func line(_ text: String, font: UIFont? = nil, color: UIColor = .label,
                             indent: CGFloat = 0, first: CGFloat? = nil, after: CGFloat = 2,
                             background: UIColor? = nil) -> NSMutableAttributedString {
        var attrs: [NSAttributedString.Key: Any] = [
            .font: font ?? mono(), .foregroundColor: color,
            .paragraphStyle: style(indent: indent, first: first, after: after),
        ]
        if let background { attrs[.backgroundColor] = background }
        return NSMutableAttributedString(string: text, attributes: attrs)
    }

    static func build(turns: [TranscriptTurn], expandedThinking: Set<String>, expandedTools: Set<String>, dark: Bool) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for turn in turns {
            switch turn.turnType {
            case .user:
                if let c = turn.content, !c.isEmpty {
                    out.append(line("> " + c + "\n", color: .label, indent: 14, first: 0, after: 8,
                                    background: .secondarySystemFill))
                }
            case .system:
                if let c = turn.content, !c.isEmpty {
                    out.append(line(c + "\n", color: .secondaryLabel, after: 8))
                }
            case .approvalRequest:
                out.append(line("⚠ Approval requested\n", font: mono(.bold), color: .systemOrange))
                if let c = turn.content, !c.isEmpty { out.append(line(c + "\n", indent: 14, after: 8)) }
            case .assistant:
                appendAssistant(turn, into: out, expandedThinking, expandedTools, dark)
            }
        }
        return out
    }

    private static func appendAssistant(_ turn: TranscriptTurn, into out: NSMutableAttributedString,
                                        _ expandedThinking: Set<String>, _ expandedTools: Set<String>, _ dark: Bool) {
        if let thinking = turn.thinking, !thinking.isEmpty {
            let open = expandedThinking.contains(turn.id)
            let t = line("∴ Thinking…\n", font: italic(mono()), color: .tertiaryLabel, after: 2)
            if let url = FeedToggle.thinking(turn.id).url {
                t.addAttribute(.link, value: url, range: NSRange(location: 0, length: t.length - 1))
            }
            out.append(t)
            if open {
                out.append(line(thinking + "\n", font: italic(mono(size: 12.5)), color: .tertiaryLabel,
                                indent: 14, first: 14, after: 6))
            }
        }
        if let content = turn.content, !content.isEmpty {
            var first = true
            for seg in FeedMarkdown.segments(content) {
                switch seg {
                case .prose(let text):
                    appendProse(text, into: out, bullet: first)
                case .code(let lang, let code):
                    appendCode(lang, code, into: out, bullet: first, dark: dark)
                }
                first = false
            }
        }
        for tc in turn.toolCalls {
            appendTool(tc, into: out, open: expandedTools.contains(tc.callId))
        }
        out.append(NSAttributedString(string: "\n", attributes: [.font: mono(size: 6)]))
    }

    // MARK: Tools

    private static func appendTool(_ tc: TranscriptToolCall, into out: NSMutableAttributedString, open: Bool) {
        let dot: UIColor = tc.status == .error ? .systemRed : (tc.status == .running ? .systemYellow : .systemGreen)
        let head = NSMutableAttributedString()
        head.append(line("● ", color: dot))
        head.append(line(tc.toolName, font: mono(.bold)))
        head.append(line("(" + tc.argumentsSummary + ")\n", color: .secondaryLabel))
        head.addAttribute(.paragraphStyle, value: style(indent: 14, first: 0, after: 1),
                          range: NSRange(location: 0, length: head.length))
        if let url = FeedToggle.tool(tc.callId).url {
            head.addAttribute(.link, value: url, range: NSRange(location: 2, length: max(0, head.length - 3)))
        }
        out.append(head)
        if open, let diff = editDiff(tc) {
            out.append(diff)
        } else if open, let args = tc.fullArgumentsJson, !args.isEmpty {
            out.append(line(args + "\n", font: mono(size: 12), color: .secondaryLabel, indent: 28, first: 28, after: 2,
                            background: .secondarySystemFill))
        }
        guard let result = tc.result, !result.isEmpty else {
            out.append(NSAttributedString(string: "", attributes: [:]))
            return
        }
        let color: UIColor = tc.isError ? .systemRed : .secondaryLabel
        let lines = result.components(separatedBy: "\n")
        let shown = open ? lines : Array(lines.prefix(previewLines))
        for (i, l) in shown.enumerated() {
            out.append(line((i == 0 ? "  ⎿  " : "     ") + l + "\n", font: mono(size: 12.5), color: color,
                            indent: 40, first: 0, after: 0))
        }
        if !open, lines.count > previewLines {
            let more = line("     … +\(lines.count - previewLines) lines\n", font: mono(size: 12.5), color: .tertiaryLabel,
                            indent: 40, first: 0, after: 0)
            if let url = FeedToggle.tool(tc.callId).url {
                more.addAttribute(.link, value: url, range: NSRange(location: 0, length: more.length - 1))
            }
            out.append(more)
        }
        out.append(NSAttributedString(string: "\n", attributes: [.font: mono(size: 4)]))
    }

    /// Edit / Write tool calls as a coloured diff: removed lines red, added lines green.
    private static func editDiff(_ tc: TranscriptToolCall) -> NSAttributedString? {
        guard let json = tc.fullArgumentsJson, let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        var pairs: [(String, String)] = []
        if let o = obj["old_string"] as? String, let n = obj["new_string"] as? String { pairs = [(o, n)] }
        else if let edits = obj["edits"] as? [[String: Any]] {
            pairs = edits.compactMap { e in
                guard let o = e["old_string"] as? String, let n = e["new_string"] as? String else { return nil }
                return (o, n)
            }
        } else if let c = obj["content"] as? String { pairs = [("", c)] }
        guard !pairs.isEmpty else { return nil }
        let out = NSMutableAttributedString()
        if let file = (obj["file_path"] ?? obj["path"]) as? String {
            out.append(line(file + "\n", font: mono(.bold, size: 12), color: .secondaryLabel, indent: 28, first: 28, after: 0))
        }
        var budget = 120
        func add(_ text: String, sign: String, color: UIColor) {
            for l in text.components(separatedBy: "\n") where budget > 0 {
                budget -= 1
                out.append(line(sign + " " + l + "\n", font: mono(size: 12), color: .label, indent: 42, first: 28, after: 0,
                                background: color.withAlphaComponent(0.22)))
            }
        }
        for (o, n) in pairs {
            if !o.isEmpty { add(o, sign: "-", color: .systemRed) }
            add(n, sign: "+", color: .systemGreen)
        }
        if budget == 0 { out.append(line("… (truncated)\n", font: mono(size: 12), color: .tertiaryLabel, indent: 28, first: 28, after: 0)) }
        return out
    }

    // MARK: Prose (headings, lists, quotes, tables, rules, inline Markdown)

    private static func appendProse(_ text: String, into out: NSMutableAttributedString, bullet: Bool) {
        var lines = text.components(separatedBy: "\n")[...]
        var firstLine = bullet
        func lead() -> String {
            defer { firstLine = false }
            return firstLine ? "● " : "  "
        }
        while let raw = lines.popFirst() {
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty { out.append(line("\n", font: mono(size: 6), after: 0)); continue }
            if trimmed.hasPrefix("|") {
                var rows = [trimmed]
                while let nxt = lines.first, nxt.trimmingCharacters(in: .whitespaces).hasPrefix("|") {
                    rows.append(lines.popFirst()!.trimmingCharacters(in: .whitespaces))
                }
                out.append(line(lead(), color: .label, after: 0))
                out.append(table(rows))
                continue
            }
            if let h = trimmed.firstMatch(of: /^(#{1,6})\s+(.*)$/) {
                let level = h.1.count
                let f = mono(.bold, size: level <= 2 ? 15.5 : size)
                let m = NSMutableAttributedString(attributedString: line(lead(), color: .label, after: 0))
                m.append(inline(String(h.2), base: f, color: level <= 2 ? .label : .secondaryLabel,
                                indent: 14, first: 0, after: 4))
                m.append(NSAttributedString(string: "\n"))
                out.append(m)
                continue
            }
            if trimmed.range(of: #"^(-{3,}|\*{3,}|_{3,})$"#, options: .regularExpression) != nil {
                out.append(line("  " + String(repeating: "─", count: 24) + "\n", color: .tertiaryLabel))
                continue
            }
            let leadingSpaces = raw.prefix(while: { $0 == " " }).count
            let depth = CGFloat(leadingSpaces / 2) * 14
            if let m = trimmed.firstMatch(of: /^([-*+])\s+(.*)$/) {
                let marker = depth > 0 ? "◦ " : "• "
                let r = NSMutableAttributedString(attributedString: line((firstLine ? "● " : "  ") + marker, after: 2))
                firstLine = false
                r.append(inline(String(m.2), indent: 28 + depth, first: 0, after: 2))
                r.addAttribute(.paragraphStyle, value: style(indent: 28 + depth, first: depth, after: 2),
                               range: NSRange(location: 0, length: r.length))
                r.append(NSAttributedString(string: "\n"))
                out.append(r)
                continue
            }
            if let m = trimmed.firstMatch(of: /^(\d+[.)])\s+(.*)$/) {
                let r = NSMutableAttributedString(attributedString: line((firstLine ? "● " : "  ") + String(m.1) + " ", after: 2))
                firstLine = false
                r.append(inline(String(m.2), indent: 32 + depth, first: 0, after: 2))
                r.addAttribute(.paragraphStyle, value: style(indent: 32 + depth, first: depth, after: 2),
                               range: NSRange(location: 0, length: r.length))
                r.append(NSAttributedString(string: "\n"))
                out.append(r)
                continue
            }
            if trimmed.hasPrefix(">") {
                let body = trimmed.drop(while: { $0 == ">" || $0 == " " })
                let r = NSMutableAttributedString(attributedString: line("  ▎ ", color: .tertiaryLabel, after: 2))
                firstLine = false
                r.append(inline(String(body), color: .secondaryLabel, indent: 28, first: 0, after: 2))
                r.append(NSAttributedString(string: "\n"))
                out.append(r)
                continue
            }
            let r = NSMutableAttributedString(attributedString: line(lead(), after: 2))
            r.append(inline(trimmed, indent: 14, first: 0, after: 4))
            r.append(NSAttributedString(string: "\n"))
            out.append(r)
        }
    }

    /// A Markdown table as aligned columns (the font is monospace, so padding lines them up).
    private static func table(_ rows: [String]) -> NSAttributedString {
        let cells: [[String]] = rows.compactMap { row in
            let parts = row.split(separator: "|", omittingEmptySubsequences: false)
                .dropFirst().dropLast().map { $0.trimmingCharacters(in: .whitespaces) }
            if parts.isEmpty { return nil }
            if parts.allSatisfy({ $0.range(of: #"^:?-{2,}:?$"#, options: .regularExpression) != nil }) { return nil }
            return parts
        }
        let columns = cells.map(\.count).max() ?? 0
        var widths = [Int](repeating: 0, count: columns)
        for r in cells { for (i, c) in r.enumerated() { widths[i] = max(widths[i], c.count) } }
        let out = NSMutableAttributedString()
        for (n, r) in cells.enumerated() {
            let text = r.enumerated().map { $1.padding(toLength: widths[$0], withPad: " ", startingAt: 0) }.joined(separator: " │ ")
            out.append(line((n == 0 ? "" : "  ") + text + "\n", font: mono(n == 0 ? .bold : .regular, size: 12.5),
                            indent: 14, first: 0, after: 0, background: n == 0 ? .secondarySystemFill : nil))
            if n == 0, cells.count > 1 {
                out.append(line("  " + widths.map { String(repeating: "─", count: $0) }.joined(separator: "─┼─") + "\n",
                                font: mono(size: 12.5), color: .tertiaryLabel, indent: 14, first: 0, after: 0))
            }
        }
        return out
    }

    // MARK: Code

    private static func appendCode(_ lang: String, _ code: String, into out: NSMutableAttributedString, bullet: Bool, dark: Bool) {
        if !lang.isEmpty || bullet {
            out.append(line((bullet ? "● " : "  ") + lang + "\n", font: mono(size: 11), color: .tertiaryLabel, after: 0))
        }
        let bg = UIColor.tertiarySystemFill
        if let lines = FeedHighlightStore.lookup(lang, code, dark: dark) {
            let body = NSMutableAttributedString()
            for (i, l) in lines.enumerated() {
                if i > 0 { body.append(NSAttributedString(string: "\n")) }
                for run in l.runs {
                    let c: UIColor = run.color.map { UIColor(red: $0.red, green: $0.green, blue: $0.blue, alpha: 1) } ?? .label
                    body.append(NSAttributedString(string: run.text, attributes: [.foregroundColor: c]))
                }
            }
            body.addAttributes([.font: mono(size: 12.5), .backgroundColor: bg,
                                .paragraphStyle: style(indent: 14, first: 14, after: 6)],
                               range: NSRange(location: 0, length: body.length))
            out.append(body)
            out.append(line("\n", font: mono(size: 12.5), after: 6))
        } else {
            out.append(line(code + "\n", font: mono(size: 12.5), indent: 14, first: 14, after: 6, background: bg))
        }
    }

    // MARK: Inline Markdown

    private static func italic(_ f: UIFont) -> UIFont {
        f.fontDescriptor.withSymbolicTraits(.traitItalic).map { UIFont(descriptor: $0, size: f.pointSize) } ?? f
    }

    /// Bold, italic, `code` and strike-through as UIKit attributes; links stay plain text.
    static func inline(_ markdown: String, base: UIFont? = nil, color: UIColor = .label,
                       indent: CGFloat = 0, first: CGFloat = 0, after: CGFloat = 2) -> NSAttributedString {
        let baseFont = base ?? mono()
        let attr = FeedMarkdown.inline(markdown)
        let out = NSMutableAttributedString()
        for run in attr.runs {
            let piece = String(attr[run.range].characters)
            var font = baseFont
            var extra: [NSAttributedString.Key: Any] = [:]
            if let intent = run.inlinePresentationIntent {
                if intent.contains(.stronglyEmphasized) { font = mono(.bold, size: baseFont.pointSize) }
                if intent.contains(.emphasized) { font = italic(font) }
                if intent.contains(.code) {
                    font = mono(.regular, size: baseFont.pointSize - 0.5)
                    extra[.backgroundColor] = UIColor.secondarySystemFill
                    extra[.foregroundColor] = UIColor.systemOrange
                }
                if intent.contains(.strikethrough) { extra[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            }
            var attrs: [NSAttributedString.Key: Any] = [
                .font: font, .foregroundColor: color,
                .paragraphStyle: style(indent: indent, first: first, after: after),
            ]
            attrs.merge(extra) { $1 }
            out.append(NSAttributedString(string: piece, attributes: attrs))
        }
        return out
    }
}
#endif
