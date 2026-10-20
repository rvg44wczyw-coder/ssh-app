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
    /// Text size factor (pinch to change); `onScale` reports the new one when the gesture ends.
    let textScale: CGFloat
    let onScale: (CGFloat) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextView {
        let storage = NSTextStorage()
        let layout = FeedLayoutManager()
        storage.addLayoutManager(layout)
        let container = NSTextContainer(size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layout.addTextContainer(container)
        let tv = UITextView(frame: .zero, textContainer: container)
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
        let pinch = UIPinchGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.pinched(_:)))
        pinch.delegate = context.coordinator
        tv.addGestureRecognizer(pinch)
        return tv
    }

    func updateUIView(_ tv: UITextView, context: Context) {
        let c = context.coordinator
        c.parent = self
        let width = (tv.bounds.width > 0 ? tv.bounds.width : UIScreen.main.bounds.width).rounded()
        let key = Coordinator.Inputs(turns: turns, thinking: expandedThinking, tools: expandedTools,
                                     dark: dark, highlight: highlightVersion, scale: textScale, width: width)
        if findToken != c.lastFind {
            c.lastFind = findToken
            tv.findInteraction?.presentFindNavigator(showingReplace: false)
        }
        var jump = false
        if scrollToken != c.lastToken {
            c.lastToken = scrollToken
            jump = true
        }
        let atEnd = c.isAtBottom(tv) || c.shownInputs == nil
        if let shown = c.shownInputs, shown != key {
            // New output is an event, not a redraw: while the reader is up in the history the text
            // stays exactly as it is (the Latest button carries the news); it is applied when they
            // come back to the end or tap Latest. Folding, size or colour changes apply at once.
            if shown.sameLook(as: key) && !atEnd && !jump { return }
        } else if c.shownInputs != nil {
            if jump { c.scrollToEnd(tv) }   // nothing changed: SwiftUI only called us for another reason
            return
        }

        FeedAttributedBuilder.scale = textScale
        FeedAttributedBuilder.width = width
        let text = FeedAttributedBuilder.build(
            turns: turns, expandedThinking: expandedThinking, expandedTools: expandedTools, dark: dark)
        let offset = tv.contentOffset
        let selection = tv.selectedRange
        c.lastText = text
        c.shownInputs = key
        // Replacing the text resets the scroll position for a moment: that must not flip
        // "at the bottom" (and with it the Latest button) before the position is put back.
        c.updating = true
        tv.attributedText = text
        if selection.location + selection.length <= text.length { tv.selectedRange = selection }
        // Lay the whole text out first: scrolling to an estimated end is what made the view jump.
        tv.layoutManager.ensureLayout(for: tv.textContainer)
        tv.layoutIfNeeded()
        if atEnd || jump {
            c.scrollToEnd(tv)
        } else {
            tv.setContentOffset(offset, animated: false)
        }
        c.updating = false
        c.report(tv)
    }

    final class Coordinator: NSObject, UITextViewDelegate, UIGestureRecognizerDelegate {
        var parent: FeedTextView
        private var pinchBase: CGFloat = 1

        static let range: ClosedRange<CGFloat> = 0.75...1.8

        /// Selection and scrolling keep working; the pinch only adds the size change.
        func gestureRecognizer(_ g: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

        @objc func pinched(_ g: UIPinchGestureRecognizer) {
            switch g.state {
            case .began: pinchBase = parent.textScale
            case .ended:
                let next = min(max(pinchBase * g.scale, Self.range.lowerBound), Self.range.upperBound)
                if abs(next - parent.textScale) > 0.02 { parent.onScale(next) }
            default: break
            }
        }
        struct Inputs: Equatable {
            let turns: [TranscriptTurn]
            let thinking: Set<String>
            let tools: Set<String>
            let dark: Bool
            let highlight: Int
            let scale: CGFloat
            let width: CGFloat

            /// Everything but the turns is the same: only new output differs.
            func sameLook(as o: Inputs) -> Bool {
                thinking == o.thinking && tools == o.tools && dark == o.dark
                    && highlight == o.highlight && scale == o.scale && width == o.width
            }
        }
        var shownInputs: Inputs?
        var updating = false
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
            if updating { return }
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
            // Edit / Write diffs: the old and the new text are highlighted as files of that type.
            for tc in turn.toolCalls {
                guard let (file, pairs) = FeedAttributedBuilder.editPairs(tc) else { continue }
                let lang = FeedAttributedBuilder.diffLanguage(file)
                for text in pairs.flatMap({ [$0.0, $0.1] }) {
                    let k = key(lang, text, dark)
                    guard !lang.isEmpty, !text.isEmpty, cache[k] == nil, text.utf8.count < 20_000 else { continue }
                    if let lines = await SyntaxService.shared.highlight(text: text, filename: "code." + ext(lang), dark: dark) {
                        cache[k] = lines
                        added = true
                    }
                }
            }
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

    /// Text size factor set by the pinch gesture (1 = the designed sizes); applied to every size below.
    static var scale: CGFloat = 1

    /// Width of the text view; the user's messages take the right part of it, the agent's the whole line.
    static var width: CGFloat = 390

    private static func mono(_ weight: UIFont.Weight = .regular, size: CGFloat = size) -> UIFont {
        UIFont.monospacedSystemFont(ofSize: size * scale, weight: weight)
    }

    /// For sizes that were already scaled (taken from an existing font).
    private static func monoExact(_ weight: UIFont.Weight = .regular, size: CGFloat) -> UIFont {
        UIFont.monospacedSystemFont(ofSize: size, weight: weight)
    }

    /// Space between a block's frame and its text.
    static let pad: CGFloat = 8
    /// Air between blocks: enough for one folded service line ("Worked for 8s") to sit between two
    /// blocks without touching them; tool calls of one reply stand closer together (but a frame reaches
    /// 5 pt past its text on each side, so the gap between two frames must stay above 10 pt).
    private static let blockGap: CGFloat = 14
    private static let toolGap: CGFloat = 9
    private static var blockCounter = 0

    /// Marks the text added since `from` as one framed block (`a` reply, `t` tool call, `u<margin>` your message).
    private static func block(_ out: NSMutableAttributedString, from: Int, kind: String) {
        guard out.length > from else { return }
        blockCounter += 1
        out.addAttribute(FeedLayoutManager.key, value: "\(kind):\(blockCounter)",
                         range: NSRange(location: from, length: out.length - from))
    }

    private static func gap(_ out: NSMutableAttributedString, _ size: CGFloat = 8) {
        out.append(NSAttributedString(string: "\n", attributes: [.font: mono(size: size)]))
    }

    private static func clip(_ s: String, _ n: Int) -> String {
        s.count <= n ? s : String(s.prefix(n)) + "…"
    }

    private static func style(indent: CGFloat = 0, first: CGFloat? = nil, after: CGFloat = 2) -> NSMutableParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineSpacing = 2
        p.paragraphSpacing = after
        p.firstLineHeadIndent = (first ?? indent) + pad
        p.headIndent = indent + pad
        p.tailIndent = -pad
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
        blockCounter = 0
        var i = 0
        while i < turns.count {
            let turn = turns[i]
            switch turn.turnType {
            case .user:
                i += 1
                if let c = turn.content, !c.isEmpty {
                    if out.length > 0 { gap(out, blockGap) }   // air between the agent's text and your message
                    appendUser(turn, c, into: out, expandedTools)
                }
            case .system:
                // Service entries (hooks, attachments, durations, checkpoints) in a row: ONE folded line.
                var run = [turn]
                i += 1
                while i < turns.count, turns[i].turnType == .system { run.append(turns[i]); i += 1 }
                appendService(run, into: out, expandedTools)
            case .approvalRequest:
                i += 1
                out.append(line("⚠ Approval requested\n", font: mono(.bold), color: .systemOrange))
                if let c = turn.content, !c.isEmpty { out.append(line(c + "\n", indent: 14, after: 8)) }
            case .assistant:
                i += 1
                let next = i < turns.count ? turns[i] : nil
                let nextIsTool = next.map { $0.turnType == .assistant && ($0.content ?? "").isEmpty
                    && $0.thinking == nil && !$0.toolCalls.isEmpty } ?? false
                appendAssistant(turn, into: out, expandedThinking, expandedTools, dark, nextIsTool: nextIsTool)
            }
        }
        return out
    }

    /// A run of service entries as one dim line: "▸ Hook · 3× Attachment · Worked for 8s".
    private static func appendService(_ run: [TranscriptTurn], into out: NSMutableAttributedString,
                                      _ expandedTools: Set<String>) {
        let id = "svc:" + (run.first?.id ?? "")
        let open = expandedTools.contains(id)
        var order: [String] = []
        var counts: [String: Int] = [:]
        for t in run {
            let name = t.label ?? "Service"
            let key = name.hasPrefix("Worked for") ? "Worked" : name
            if counts[key] == nil { order.append(key) }
            counts[key, default: 0] += 1
        }
        let names = order.map { counts[$0]! > 1 ? "\(counts[$0]!)× \($0)" : $0 }
        var shown = names.prefix(4).joined(separator: " · ")
        if names.count > 4 { shown += " +\(names.count - 4)" }
        let head = line((open ? "▾ " : "▸ ") + shown + "\n", font: mono(.medium, size: 12), color: .tertiaryLabel, after: 16)
        if let url = FeedToggle.tool(id).url {
            head.addAttribute(.link, value: url, range: NSRange(location: 0, length: head.length - 1))
        }
        out.append(head)
        guard open else { return }
        for t in run {
            out.append(line((t.label ?? "Service") + "\n", font: mono(.semibold, size: 12), color: .tertiaryLabel,
                            indent: 14, first: 14, after: 3))
            if let c = t.content?.trimmingCharacters(in: .whitespacesAndNewlines), !c.isEmpty {
                for part in FeedXML.parts(c) {
                    switch part {
                    case .text(let text):
                        out.append(line(text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n", font: mono(size: 12),
                                        color: .secondaryLabel, indent: 14, first: 14, after: 14,
                                        background: .secondarySystemFill))
                    case .tags(let tags):
                        for tag in tags {
                            out.append(line(FeedXML.title(tag.name) + "\n", font: mono(.semibold, size: 11.5), color: .tertiaryLabel,
                                            indent: 22, first: 22, after: 3))
                            out.append(line(tag.body.trimmingCharacters(in: .whitespacesAndNewlines) + "\n", font: mono(size: 12),
                                            color: .secondaryLabel, indent: 22, first: 22, after: 12,
                                            background: .secondarySystemFill))
                        }
                    }
                }
            }
        }
    }

    private static func appendAssistant(_ turn: TranscriptTurn, into out: NSMutableAttributedString,
                                        _ expandedThinking: Set<String>, _ expandedTools: Set<String>, _ dark: Bool,
                                        nextIsTool: Bool = false) {
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
            let from = out.length
            defer { block(out, from: from, kind: "a"); gap(out, blockGap) }
            for (i, part) in FeedXML.parts(content).enumerated() {
                switch part {
                case .text(let text):
                    for seg in FeedMarkdown.segments(text) {
                        switch seg {
                        case .prose(let text):
                            appendProse(text, into: out, bullet: first)
                        case .code(let lang, let code):
                            appendCode(lang, code, into: out, bullet: first, dark: dark)
                        }
                        first = false
                    }
                case .tags(let tags):
                    appendTags(tags, id: "\(turn.id)#\(i)", into: out, open: expandedTools.contains("\(turn.id)#\(i)"),
                               indent: 0)
                }
            }
        }
        for (k, tc) in turn.toolCalls.enumerated() {
            let last = k == turn.toolCalls.count - 1
            appendTool(tc, into: out, open: expandedTools.contains(tc.callId),
                       full: expandedTools.contains(tc.callId + "+"), dark: dark,
                       tight: !last || nextIsTool)
        }
        if turn.content?.isEmpty ?? true { gap(out, 4) }
    }

    // MARK: User bubble and XML tags

    /// The user's message as a chat bubble pinned to the right (about three quarters of the width).
    private static func appendUser(_ turn: TranscriptTurn, _ content: String, into out: NSMutableAttributedString,
                                   _ expandedTools: Set<String>) {
        // Your message is a bubble pinned to the right (about 3/4 of the width); the text inside starts at its left edge.
        let margin = (width * 0.25).rounded()
        for (i, part) in FeedXML.parts(content).enumerated() {
            switch part {
            case .text(let text):
                let body = text.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !body.isEmpty else { continue }
                let p = style(indent: margin, first: margin, after: 2)
                let from = out.length
                out.append(NSAttributedString(string: body + "\n", attributes: [
                    .font: mono(), .foregroundColor: UIColor.label, .paragraphStyle: p,
                ]))
                block(out, from: from, kind: "u\(Int(margin))")
                gap(out, blockGap + 2)
            case .tags(let tags):
                let id = "\(turn.id)#\(i)"
                appendTags(tags, id: id, into: out, open: expandedTools.contains(id), indent: margin, right: false)
            }
        }
    }

    /// A run of XML-ish blocks (`<system-reminder>…</system-reminder>`, `<command-name>…`) as ONE dim line
    /// with the tag names; tapping it shows each block under its own name.
    private static func appendTags(_ tags: [(name: String, body: String)], id: String, into out: NSMutableAttributedString,
                                   open: Bool, indent: CGFloat, right: Bool = false) {
        let names = tags.map { FeedXML.title($0.name) }
        var shown = names.prefix(3).joined(separator: ", ")
        if names.count > 3 { shown += " +\(names.count - 3)" }
        let head = line((open ? "▾ " : "▸ ") + shown + "\n", font: mono(.medium, size: 12), color: .secondaryLabel,
                        indent: indent, first: indent, after: 4)
        if right, let ps = head.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSMutableParagraphStyle {
            ps.alignment = .right
        }
        if let url = FeedToggle.tool(id).url {
            head.addAttribute(.link, value: url, range: NSRange(location: 0, length: head.length - 1))
        }
        out.append(head)
        guard open else { return }
        for t in tags {
            out.append(line(FeedXML.title(t.name) + "\n", font: mono(.semibold, size: 12), color: .tertiaryLabel,
                            indent: indent + 14, first: indent + 14, after: 0))
            let body = t.body.trimmingCharacters(in: .whitespacesAndNewlines)
            if !body.isEmpty {
                out.append(line(body + "\n", font: mono(size: 12), color: .secondaryLabel,
                                indent: indent + 14, first: indent + 14, after: 4, background: .secondarySystemFill))
            }
        }
    }

    /// A long shell command as a tree: steps joined by `&&` / `||` / `;` are siblings, a pipeline hangs
    /// under its first command, a here-document is a folded body.
    private static func appendShellTree(_ nodes: [ShellTree.Node], into out: NSMutableAttributedString, open: Bool) {
        let limit = open ? Int.max : 12
        var shown = 0
        func emit(_ text: String, _ prefix: String, _ cont: String, color: UIColor = .secondaryLabel) {
            out.append(line(prefix + text + "\n", font: mono(size: 12.5), color: color,
                            indent: 28 + CGFloat(cont.count) * 7.5, first: 14, after: 0))
            shown += 1
        }
        for (i, n) in nodes.enumerated() {
            if shown >= limit {
                out.append(line("  … +\(ShellTree.stepCount(Array(nodes[i...]))) more steps\n", font: mono(size: 12.5),
                                color: .tertiaryLabel, indent: 28, first: 14, after: 0))
                break
            }
            let last = i == nodes.count - 1
            let branch = last ? "└─ " : "├─ "
            let trunk = last ? "   " : "│  "
            emit((n.op.isEmpty ? "" : n.op + " ") + clip(n.text, open ? 2000 : 140), branch, trunk)
            let kids = n.pipe.map { ("| " + clip($0, open ? 2000 : 120), UIColor.secondaryLabel) }
            for (j, k) in kids.enumerated() {
                let lastKid = j == kids.count - 1 && n.heredoc.isEmpty
                emit(k.0, trunk + (lastKid ? "└─ " : "├─ "), trunk + "   ")
            }
            if !n.heredoc.isEmpty {
                let body = n.heredoc
                let shownBody = open ? body : Array(body.prefix(3))
                for (j, l) in shownBody.enumerated() {
                    let lastLine = j == shownBody.count - 1 && (open || body.count <= 3)
                    emit(clip(l, open ? 2000 : 160), trunk + (lastLine ? "└─ " : "├─ "), trunk + "   ", color: .tertiaryLabel)
                }
                if !open, body.count > 3 {
                    emit("… +\(body.count - 3) lines", trunk + "└─ ", trunk + "   ", color: .tertiaryLabel)
                }
            }
        }
    }

    // MARK: Tools

    /// Tool calls open in stages (a tap goes on to the next one): shell commands start closed -
    /// the header only; stage 1 adds the steps as a tree and a short result; stage 2 everything.
    /// Other tools have two stages (`open`).
    private static func appendTool(_ tc: TranscriptToolCall, into out: NSMutableAttributedString, open: Bool,
                                   full: Bool, dark: Bool, tight: Bool) {
        let from = out.length
        appendToolBody(tc, into: out, open: open, full: full, dark: dark)
        block(out, from: from, kind: "t")
        gap(out, tight ? toolGap : blockGap)
    }

    private static func appendToolBody(_ tc: TranscriptToolCall, into out: NSMutableAttributedString, open: Bool,
                                       full: Bool, dark: Bool) {
        let dot: UIColor = tc.status == .error ? .systemRed : (tc.status == .running ? .systemYellow : .systemGreen)
        let shell = ShellTree.isShell(tc.toolName)
        let tree = ShellTree.nodes(for: tc, always: shell && open)
        let steps = ShellTree.nodes(for: tc, always: false)
        let head = NSMutableAttributedString()
        head.append(line("● ", color: dot))
        head.append(line(tc.toolName, font: mono(.bold)))
        if let steps {
            head.append(line("(\(ShellTree.stepCount(steps)) steps)", color: .secondaryLabel))
        } else {
            head.append(line("(" + clip(tc.argumentsSummary, shell && !open ? 34 : 160) + ")", color: .secondaryLabel))
        }
        if shell { head.append(line(open ? (full ? " ▾" : " ▸") : " ▸", color: .tertiaryLabel)) }
        head.append(line("\n"))
        head.addAttribute(.paragraphStyle, value: style(indent: 14, first: 0, after: 1),
                          range: NSRange(location: 0, length: head.length))
        if let url = FeedToggle.tool(tc.callId).url {
            head.addAttribute(.link, value: url, range: NSRange(location: 2, length: max(0, head.length - 3)))
        }
        out.append(head)
        if shell && !open {
            return
        }
        if let tree {
            appendShellTree(tree, into: out, open: full)
        } else if open, let diff = editDiff(tc, dark: dark) {
            out.append(diff)
        } else if open, !shell, let args = tc.fullArgumentsJson, !args.isEmpty {
            out.append(line(args + "\n", font: mono(size: 12), color: .secondaryLabel, indent: 28, first: 28, after: 2,
                            background: .secondarySystemFill))
        }
        guard let result = tc.result, !result.isEmpty else {
            out.append(NSAttributedString(string: "", attributes: [:]))
            return
        }
        let color: UIColor = tc.isError ? .systemRed : .secondaryLabel
        let lines = result.components(separatedBy: "\n")
        let all = shell ? full : open
        let shown = all ? lines : Array(lines.prefix(previewLines))
        for (i, l) in shown.enumerated() {
            out.append(line((i == 0 ? "  ⎿  " : "     ") + clip(l, all ? 2000 : 240) + "\n", font: mono(size: 12.5), color: color,
                            indent: 40, first: 0, after: 0))
        }
        if !all, lines.count > previewLines {
            let more = line("     … +\(lines.count - previewLines) lines\n", font: mono(size: 12.5), color: .tertiaryLabel,
                            indent: 40, first: 0, after: 0)
            if let url = FeedToggle.tool(tc.callId).url {
                more.addAttribute(.link, value: url, range: NSRange(location: 0, length: more.length - 1))
            }
            out.append(more)
        }
    }

    /// Edit / Write tool calls as a coloured diff: removed lines red, added lines green.
    /// The file and the (old, new) texts of an Edit / MultiEdit / Write call, if it is one.
    static func editPairs(_ tc: TranscriptToolCall) -> (file: String?, pairs: [(String, String)])? {
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
        return ((obj["file_path"] ?? obj["path"]) as? String, pairs)
    }

    /// The syntax language key (file extension) used for a diff of `file`.
    static func diffLanguage(_ file: String?) -> String {
        guard let file, let dot = file.lastIndex(of: "."), dot != file.startIndex else { return "" }
        let e = file[file.index(after: dot)...].lowercased()
        return e.count <= 8 && e.allSatisfy({ $0.isLetter || $0.isNumber }) ? e : ""
    }

    private static func editDiff(_ tc: TranscriptToolCall, dark: Bool) -> NSAttributedString? {
        guard let (file, pairs) = editPairs(tc) else { return nil }
        let lang = diffLanguage(file)
        let out = NSMutableAttributedString()
        if let file {
            out.append(line(file + "\n", font: mono(.bold, size: 12), color: .secondaryLabel, indent: 28, first: 28, after: 0))
        }
        var budget = 120
        func add(_ text: String, sign: String, color: UIColor) {
            let colored = lang.isEmpty ? nil : FeedHighlightStore.lookup(lang, text, dark: dark)
            for (i, l) in text.components(separatedBy: "\n").enumerated() where budget > 0 {
                budget -= 1
                let row = line(sign + " " + l + "\n", font: mono(size: 12), color: .label, indent: 42, first: 28, after: 0,
                               background: color.withAlphaComponent(0.22))
                // Syntax colours for the code part of the line (offset 2 skips "+ " / "- ").
                if let colored, i < colored.count {
                    var at = 2
                    for run in colored[i].runs {
                        let length = (run.text as NSString).length
                        if let c = run.color, at + length <= row.length {
                            row.addAttribute(.foregroundColor, value: UIColor(red: c.red, green: c.green, blue: c.blue, alpha: 1),
                                             range: NSRange(location: at, length: length))
                        }
                        at += length
                    }
                }
                out.append(row)
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
                if intent.contains(.stronglyEmphasized) { font = monoExact(.bold, size: baseFont.pointSize) }
                if intent.contains(.emphasized) { font = italic(font) }
                if intent.contains(.code) {
                    font = monoExact(.regular, size: baseFont.pointSize - 0.5)
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


/// Splits a message into plain text and runs of `<tag>…</tag>` blocks (system reminders, slash-command
/// echoes, IDE context). Tags that are never closed stay in the text.
enum FeedXML {
    enum Part {
        case text(String)
        case tags([(name: String, body: String)])
    }

    /// Blocks the agent and its tools wrap their service text in, with the name shown instead of the tag.
    /// Any other tag is left alone: ordinary text may mention `<path>` or `<div>`.
    private static let titles: [String: String] = [
        "system-reminder": "System reminder",
        "command-name": "Command",
        "command-message": "Command message",
        "command-args": "Arguments",
        "local-command-stdout": "Command output",
        "local-command-stderr": "Command errors",
        "local-command-caveat": "Command note",
        "bash-input": "Shell command",
        "bash-stdout": "Shell output",
        "bash-stderr": "Shell errors",
        "task-notification": "Background task",
        "task-id": "Task id",
        "tool-use-id": "Tool call id",
        "output-file": "Output file",
        "status": "Status",
        "summary": "Summary",
        "result": "Result",
        "ide_opened_file": "Opened in IDE",
        "ide_selection": "Selection in IDE",
        "user-prompt-submit-hook": "Hook",
        "function_results": "Tool results",
        "available-deferred-tools": "Deferred tools",
        "persisted-output": "Saved output",
        "analysis": "Analysis",
        "scratchpad": "Notes",
    ]

    static func title(_ name: String) -> String { titles[name] ?? name }

    private static let regex = try? NSRegularExpression(pattern: "<([A-Za-z][A-Za-z0-9_.:-]*)>([\\s\\S]*?)</\\1>")

    static func parts(_ text: String) -> [Part] {
        let ns = text as NSString
        guard let regex, text.contains("</") else { return [.text(text)] }
        var parts: [Part] = []
        var cursor = 0
        var run: [(name: String, body: String)] = []
        func flush() { if !run.isEmpty { parts.append(.tags(run)); run = [] } }
        for m in regex.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            guard titles[ns.substring(with: m.range(at: 1))] != nil, m.range.location >= cursor else { continue }
            let gap = ns.substring(with: NSRange(location: cursor, length: m.range.location - cursor))
            if !gap.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                flush()
                parts.append(.text(gap))
            }
            run.append((ns.substring(with: m.range(at: 1)), ns.substring(with: m.range(at: 2))))
            cursor = m.range.location + m.range.length
        }
        flush()
        if cursor < ns.length {
            let rest = ns.substring(from: cursor)
            if !rest.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { parts.append(.text(rest)) }
        }
        return parts.isEmpty ? [.text(text)] : parts
    }
}


/// Splits a shell command into the steps the agent chained, for the feed's tree view.
enum ShellTree {
    struct Node {
        var op: String          // "", "&&", "||", ";" — how this step follows the previous one
        var text: String
        var pipe: [String] = []  // commands piped from this one
        var heredoc: [String] = []
    }

    static func stepCount(_ nodes: [Node]) -> Int { nodes.reduce(0) { $0 + 1 + $1.pipe.count } }

    static func isShell(_ tool: String) -> Bool {
        ["Bash", "run_command", "shell", "execute_command"].contains(tool)
    }

    /// The tree for a shell tool call, or nil when the command is a single plain step
    /// (`always` returns that single step too).
    static func nodes(for tc: TranscriptToolCall, always: Bool) -> [Node]? {
        guard isShell(tc.toolName), let json = tc.fullArgumentsJson,
              let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let cmd = (obj["command"] ?? obj["CommandLine"]) as? String
        else { return nil }
        let nodes = parse(cmd)
        if nodes.isEmpty { return nil }
        return always || stepCount(nodes) > 1 || nodes.contains(where: { !$0.heredoc.isEmpty }) ? nodes : nil
    }

    static func parse(_ cmd: String) -> [Node] {
        let c = Array(cmd.replacingOccurrences(of: "\\\n", with: " "))
        var nodes: [Node] = []
        var cur = ""
        var op = ""
        var pendingPipe = false
        var heredocs: [String] = []
        var depth = 0
        var quote: Character?
        var i = 0

        func flush(next: String) {
            let t = cur.trimmingCharacters(in: .whitespaces)
            if !t.isEmpty {
                if pendingPipe, !nodes.isEmpty { nodes[nodes.count - 1].pipe.append(t) }
                else { nodes.append(Node(op: op, text: t)) }
            }
            cur = ""
            if next == "|" { pendingPipe = true } else { pendingPipe = false; op = t.isEmpty ? op : next }
            if next != "|" && t.isEmpty && nodes.isEmpty { op = "" }
        }

        while i < c.count {
            let ch = c[i]
            if let q = quote {
                cur.append(ch)
                if ch == "\\", q == "\"", i + 1 < c.count { cur.append(c[i + 1]); i += 2; continue }
                if ch == q { quote = nil }
                i += 1; continue
            }
            switch ch {
            case "'", "\"", "`": quote = ch; cur.append(ch)
            case "\\": cur.append(ch); if i + 1 < c.count { cur.append(c[i + 1]); i += 1 }
            case "(", "{": depth += 1; cur.append(ch)
            case ")", "}": depth = max(0, depth - 1); cur.append(ch)
            case "<" where depth == 0 && i + 1 < c.count && c[i + 1] == "<" && !(i + 2 < c.count && c[i + 2] == "<"):
                var j = i + 2
                if j < c.count, c[j] == "-" { j += 1 }
                while j < c.count, c[j] == " " { j += 1 }
                var word = ""
                let q: Character? = (j < c.count && (c[j] == "'" || c[j] == "\"")) ? c[j] : nil
                if q != nil { j += 1 }
                while j < c.count, c[j].isLetter || c[j].isNumber || c[j] == "_" { word.append(c[j]); j += 1 }
                if q != nil, j < c.count { j += 1 }
                if !word.isEmpty { heredocs.append(word) }
                cur += String(c[i..<min(j, c.count)])
                i = j; continue
            case "\n" where depth == 0:
                if !heredocs.isEmpty {
                    // consume the body up to each delimiter line
                    var lines: [String] = []
                    var k = i + 1
                    var rest = heredocs
                    var line = ""
                    while k < c.count, !rest.isEmpty {
                        if c[k] == "\n" {
                            if line.trimmingCharacters(in: .whitespaces) == rest[0] { rest.removeFirst() } else { lines.append(line) }
                            line = ""
                        } else { line.append(c[k]) }
                        k += 1
                    }
                    if !line.isEmpty, !rest.isEmpty, line.trimmingCharacters(in: .whitespaces) != rest[0] { lines.append(line) }
                    let t = cur.trimmingCharacters(in: .whitespaces)
                    nodes.append(Node(op: op, text: t, heredoc: lines))
                    cur = ""; op = ""; pendingPipe = false; heredocs = []
                    i = k; continue
                }
                flush(next: "")
            case "&" where depth == 0 && i + 1 < c.count && c[i + 1] == "&":
                flush(next: "&&"); i += 2; continue
            case "|" where depth == 0 && i + 1 < c.count && c[i + 1] == "|":
                flush(next: "||"); i += 2; continue
            case "|" where depth == 0:
                flush(next: "|")
            case ";" where depth == 0:
                flush(next: ";")
            default: cur.append(ch)
            }
            i += 1
        }
        flush(next: "")
        return nodes
    }
}


/// Draws the frames of the feed's blocks behind the text: your message (a bubble pinned to the
/// right), the agent's reply (a framed card) and tool calls (a tinted box). The builder marks a
/// block's text with `FeedLayoutManager.key` = "<kind>:<n>"; one block is one frame.
final class FeedLayoutManager: NSLayoutManager {
    static let key = NSAttributedString.Key("feedBlock")

    override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
        guard let storage = textStorage, let container = textContainers.first, storage.length > 0 else { return }
        let visible = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        var at = visible.location
        while at < NSMaxRange(visible) {
            var run = NSRange(location: 0, length: 0)
            let tag = storage.attribute(Self.key, at: at, longestEffectiveRange: &run,
                                        in: NSRange(location: 0, length: storage.length)) as? String
            at = max(NSMaxRange(run), at + 1)
            guard let tag, let kind = tag.split(separator: ":").first else { continue }
            let glyphs = glyphRange(forCharacterRange: run, actualCharacterRange: nil)
            var rect = CGRect.null
            enumerateLineFragments(forGlyphRange: glyphs) { _, used, _, _, _ in rect = rect.union(used) }
            guard !rect.isNull else { continue }
            let width = container.size.width
            var frame = CGRect(x: 1, y: rect.minY - 5, width: width - 2, height: rect.height + 10)
            let fill: UIColor, stroke: UIColor
            switch kind.first {
            case "u":
                let margin = CGFloat(Int(kind.dropFirst()) ?? 0)
                frame = CGRect(x: max(1, margin - 2), y: frame.minY, width: width - max(1, margin - 2) - 1, height: frame.height)
                fill = UIColor.tintColor.withAlphaComponent(0.14)
                stroke = UIColor.tintColor.withAlphaComponent(0.45)
            case "t":
                fill = UIColor.secondarySystemFill.withAlphaComponent(0.5)
                stroke = UIColor.separator.withAlphaComponent(0.6)
            default:
                fill = UIColor.secondarySystemBackground.withAlphaComponent(0.35)
                stroke = UIColor.separator
            }
            frame = frame.offsetBy(dx: origin.x, dy: origin.y)
            let path = UIBezierPath(roundedRect: frame, cornerRadius: 10)
            fill.setFill()
            path.fill()
            stroke.setStroke()
            path.lineWidth = 1
            path.stroke()
        }
    }
}
