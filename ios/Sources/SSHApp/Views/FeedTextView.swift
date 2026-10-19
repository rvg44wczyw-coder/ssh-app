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

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextView {
        let tv = UITextView()
        tv.isEditable = false
        tv.isSelectable = true
        tv.backgroundColor = .clear
        tv.alwaysBounceVertical = true
        tv.keyboardDismissMode = .interactive
        tv.textContainerInset = UIEdgeInsets(top: 12, left: 12, bottom: 16, right: 12)
        tv.adjustsFontForContentSizeCategory = true
        tv.linkTextAttributes = [.foregroundColor: UIColor.secondaryLabel]
        tv.delegate = context.coordinator
        return tv
    }

    func updateUIView(_ tv: UITextView, context: Context) {
        let c = context.coordinator
        c.parent = self
        let text = FeedAttributedBuilder.build(
            turns: turns, expandedThinking: expandedThinking, expandedTools: expandedTools)
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

        func textView(_ textView: UITextView, shouldInteractWith URL: URL, in characterRange: NSRange,
                      interaction: UITextItemInteraction) -> Bool {
            if interaction == .invokeDefaultAction, let toggle = FeedToggle(url: URL) {
                parent.onToggle(toggle)
            }
            return false
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

/// Builds the feed's attributed text. Pure: same turns and expansion state, same text.
enum FeedAttributedBuilder {
    static func build(turns: [TranscriptTurn], expandedThinking: Set<String>, expandedTools: Set<String>) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for (i, turn) in turns.enumerated() {
            if i > 0 { out.append(plain("\n", size: 8)) }
            out.append(header(turn))
            if let thinking = turn.thinking, !thinking.isEmpty {
                let open = expandedThinking.contains(turn.id)
                let steps = thinking.components(separatedBy: "\n").count
                out.append(toggleLine("\(open ? "▾" : "▸") Thinking (\(steps) steps)", FeedToggle.thinking(turn.id)))
                if open { out.append(block(thinking, mono: 12, color: .secondaryLabel)) }
            }
            if let content = turn.content, !content.isEmpty {
                out.append(contentText(content))
            }
            for tc in turn.toolCalls {
                let open = expandedTools.contains(tc.callId)
                let mark = tc.status == .running ? "…" : (tc.status == .error ? "✗" : "✓")
                let title = "\(open ? "▾" : "▸") \(tc.toolName)  \(tc.argumentsSummary)  \(mark)"
                out.append(toggleLine(title, FeedToggle.tool(tc.callId), bold: tc.toolName.count))
                if open {
                    if let args = tc.fullArgumentsJson, !args.isEmpty {
                        out.append(block(args, mono: 11, color: .label))
                    }
                    if let res = tc.result, !res.isEmpty {
                        out.append(block(res, mono: 11, color: tc.isError ? .systemRed : .label))
                    }
                }
            }
        }
        return out
    }

    private static func style(spacingAfter: CGFloat = 4, indent: CGFloat = 0) -> NSMutableParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineSpacing = 2
        p.paragraphSpacing = spacingAfter
        p.firstLineHeadIndent = indent
        p.headIndent = indent
        return p
    }

    private static func plain(_ s: String, size: CGFloat) -> NSAttributedString {
        NSAttributedString(string: s, attributes: [.font: UIFont.systemFont(ofSize: size)])
    }

    private static func header(_ turn: TranscriptTurn) -> NSAttributedString {
        let (name, color): (String, UIColor) = {
            switch turn.turnType {
            case .user: return ("You", .systemBlue)
            case .assistant: return ("Agent", .systemPurple)
            case .approvalRequest: return ("Approval request", .systemOrange)
            case .system: return ("System", .secondaryLabel)
            }
        }()
        var text = name
        if let t = turn.timestamp, t.contains("T"), let time = t.components(separatedBy: "T").last {
            text += "  " + String(time.prefix(8))
        }
        if turn.status == .error { text += "  ✗" }
        return NSAttributedString(string: text + "\n", attributes: [
            .font: UIFont.systemFont(ofSize: 12, weight: .bold),
            .foregroundColor: color,
            .paragraphStyle: style(spacingAfter: 2),
        ])
    }

    private static func toggleLine(_ text: String, _ toggle: FeedToggle, bold: Int = 0) -> NSAttributedString {
        let s = NSMutableAttributedString(string: text + "\n", attributes: [
            .font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: UIColor.secondaryLabel,
            .paragraphStyle: style(spacingAfter: 2, indent: 8),
        ])
        if bold > 0, s.length >= bold + 2 {
            s.addAttribute(.font, value: UIFont.monospacedSystemFont(ofSize: 12, weight: .bold),
                           range: NSRange(location: 2, length: bold))
        }
        if let url = toggle.url {
            s.addAttribute(.link, value: url, range: NSRange(location: 0, length: max(0, s.length - 1)))
        }
        return s
    }

    private static func block(_ text: String, mono: CGFloat, color: UIColor) -> NSAttributedString {
        NSAttributedString(string: text + "\n", attributes: [
            .font: UIFont.monospacedSystemFont(ofSize: mono, weight: .regular),
            .foregroundColor: color,
            .backgroundColor: UIColor.secondarySystemFill,
            .paragraphStyle: style(spacingAfter: 4, indent: 8),
        ])
    }

    private static func contentText(_ content: String) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for seg in FeedMarkdown.segments(content) {
            switch seg {
            case .prose(let p):
                out.append(inline(p))
                out.append(NSAttributedString(string: "\n"))
            case .code(let lang, let code):
                if !lang.isEmpty {
                    out.append(NSAttributedString(string: lang + "\n", attributes: [
                        .font: UIFont.systemFont(ofSize: 10, weight: .medium),
                        .foregroundColor: UIColor.secondaryLabel,
                        .paragraphStyle: style(spacingAfter: 0, indent: 8),
                    ]))
                }
                out.append(block(code, mono: 13, color: .label))
            }
        }
        return out
    }

    /// Inline Markdown (bold, italic, code, links shown as text) as UIKit attributes.
    static func inline(_ markdown: String) -> NSAttributedString {
        let base = UIFont.systemFont(ofSize: 15)
        let attr = FeedMarkdown.inline(markdown)
        let out = NSMutableAttributedString()
        for run in attr.runs {
            let piece = String(attr[run.range].characters)
            var font = base
            var extra: [NSAttributedString.Key: Any] = [:]
            if let intent = run.inlinePresentationIntent {
                var traits: UIFontDescriptor.SymbolicTraits = []
                if intent.contains(.stronglyEmphasized) { traits.insert(.traitBold) }
                if intent.contains(.emphasized) { traits.insert(.traitItalic) }
                if let d = base.fontDescriptor.withSymbolicTraits(traits) { font = UIFont(descriptor: d, size: 15) }
                if intent.contains(.code) {
                    font = UIFont.monospacedSystemFont(ofSize: 14, weight: .regular)
                    extra[.backgroundColor] = UIColor.secondarySystemFill
                }
                if intent.contains(.strikethrough) { extra[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            }
            var attrs: [NSAttributedString.Key: Any] = [
                .font: font, .foregroundColor: UIColor.label, .paragraphStyle: style(spacingAfter: 6),
            ]
            attrs.merge(extra) { $1 }
            out.append(NSAttributedString(string: piece, attributes: attrs))
        }
        return out
    }
}
#endif
