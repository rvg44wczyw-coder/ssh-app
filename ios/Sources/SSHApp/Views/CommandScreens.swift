import SshCoreBridge
import SwiftUI

/// Keys for the dialogs of the agent's slash commands. Only fixed sequences are ever sent, never
/// text from a row of the screen.
enum DialogKeys {
    static let up: [UInt8] = [0x1b, 0x5b, 0x41]
    static let down: [UInt8] = [0x1b, 0x5b, 0x42]
    static let left: [UInt8] = [0x1b, 0x5b, 0x44]
    static let right: [UInt8] = [0x1b, 0x5b, 0x43]
    static let enter: [UInt8] = [0x0d]
    static let space: [UInt8] = [0x20]

    /// Arrow presses that go from the tab that is open to another one (←/→ switch tabs in these dialogs).
    static func switchTab(from: Int, to: Int) -> [[UInt8]] {
        guard from != to else { return [] }
        return Array(repeating: to > from ? right : left, count: abs(to - from))
    }

    /// Arrow presses that move the cursor from the row it is on to another one.
    static func move(from: Int, to: Int) -> [[UInt8]] {
        guard from != to else { return [] }
        return Array(repeating: to > from ? down : up, count: abs(to - from))
    }
}

/// The `/btw` answer as a card above the input: the question, a loader while Claude answers, the
/// answer as Markdown, and Copy / Close.
struct FeedAsideCard: View {
    let aside: ScreenAside
    let close: () -> Void
    let more: () -> Void

    var body: some View {
        GlassEffectContainer {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top) {
                    Image(systemName: "text.bubble").foregroundColor(.appAccent)
                    Text(verbatim: aside.question).font(.subheadline.weight(.semibold)).lineLimit(3)
                    Spacer(minLength: 0)
                }
                if aside.pending {
                    ProgressView().controlSize(.small)
                } else {
                    ScrollView {
                        Text(Self.markdown(aside.answer)).font(.callout).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 240)
                }
                HStack(spacing: 10) {
                    if !aside.pending {
                        Button { UIPasteboard.general.string = aside.answer } label: { Label("Copy", systemImage: "doc.on.doc") }
                            .buttonStyle(.glass)
                    }
                    if aside.scrollable {
                        Button(action: more) { Label("More", systemImage: "chevron.down") }.buttonStyle(.glass)
                    }
                    Spacer(minLength: 0)
                    Button(action: close) { Label("Close", systemImage: "xmark") }.buttonStyle(.glassProminent)
                }
                .font(.footnote)
            }
            .padding(12)
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 18))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private static func markdown(_ s: String) -> AttributedString {
        var text = (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
        // The answer is model output: no tappable links from it.
        for run in text.runs where run.link != nil { text[run.range].link = nil }
        return text
    }
}

/// One native screen for the full-screen dialogs of the agent's commands. What is shown is what the
/// pane showed in the last capture; a tap sends fixed keys to move the cursor and press Enter, and
/// the screen follows the next capture.
struct CommandScreenView: View {
    let dialog: ScreenDialog
    let send: ([[UInt8]]) -> Void
    let close: () -> Void

    var body: some View {
        NavigationStack {
            List {
                if !dialog.note.isEmpty {
                    Section { Text(verbatim: dialog.note).font(.footnote).foregroundColor(.secondary) }
                }
                if dialog.tabs.count > 1 {
                    Section {
                        ScrollView(.horizontal, showsIndicators: false) {
                            GlassEffectContainer(spacing: 6) {
                                HStack(spacing: 6) {
                                    ForEach(Array(dialog.tabs.enumerated()), id: \.offset) { index, tab in
                                        Button { send(DialogKeys.switchTab(from: activeTab, to: index)) } label: {
                                            Text(verbatim: tab.name).font(.footnote.weight(tab.active ? .semibold : .regular))
                                                .padding(.horizontal, 10).padding(.vertical, 5)
                                        }
                                        .buttonStyle(.plain)
                                        .glassEffect(tab.active ? .regular.tint(Color.appAccent) : .regular, in: Capsule())
                                    }
                                }
                            }
                        }
                    }
                }
                if let search = dialog.search, !search.isEmpty {
                    Section { LabeledContent("Filter", value: search) }
                }
                Section {
                    if dialog.moreAbove { Text("More above").font(.caption2).foregroundColor(.secondary) }
                    ForEach(Array(dialog.rows.enumerated()), id: \.offset) { index, row in
                        rowView(index, row)
                    }
                    if dialog.moreBelow { Text("More below: scroll in the terminal").font(.caption2).foregroundColor(.secondary) }
                }
                if dialog.kind == .model || dialog.hints.contains("Effort") {
                    Section("Effort") {
                        HStack {
                            Button { send([DialogKeys.left]) } label: { Image(systemName: "minus") }.buttonStyle(.glass)
                            Spacer()
                            Text("Adjust with ← →").font(.footnote).foregroundColor(.secondary)
                            Spacer()
                            Button { send([DialogKeys.right]) } label: { Image(systemName: "plus") }.buttonStyle(.glass)
                        }
                    }
                }
                if dialog.kind != .model, dialog.hints.contains("Change") {
                    Section("Selected setting") {
                        HStack {
                            Button { send([DialogKeys.left]) } label: { Image(systemName: "chevron.left") }.buttonStyle(.glass)
                            Spacer()
                            Text("Change the value with ← →").font(.footnote).foregroundColor(.secondary)
                            Spacer()
                            Button { send([DialogKeys.right]) } label: { Image(systemName: "chevron.right") }.buttonStyle(.glass)
                        }
                    }
                }
                if dialog.hints.contains("Page") {
                    Section {
                        HStack {
                            Button { send([DialogKeys.left]) } label: { Label("Previous page", systemImage: "chevron.left") }.buttonStyle(.glass)
                            Spacer()
                            Button { send([DialogKeys.right]) } label: { Label("Next page", systemImage: "chevron.right") }.buttonStyle(.glass)
                        }
                    }
                }
                if !dialog.hints.isEmpty {
                    Section { Text(verbatim: dialog.hints).font(.caption2).foregroundColor(.secondary) }
                }
            }
            .navigationTitle(dialog.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Close", action: close) }
                if dialog.kind == .model {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("This session") { send([Array("s".utf8)]) }
                    }
                }
            }
        }
    }

    private var activeTab: Int { dialog.tabs.firstIndex(where: \.active) ?? 0 }

    private var selectable: Bool { dialog.kind == .model || dialog.kind == .permissions || dialog.kind == .config || dialog.kind == .generic }

    @ViewBuilder
    private func rowView(_ index: Int, _ row: DialogRow) -> some View {
        let content = HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let n = row.number { Text("\(n)").font(.caption.monospacedDigit()).foregroundColor(.secondary).frame(width: 22, alignment: .trailing) }
            switch row.state {
            case .current: Image(systemName: "checkmark.circle.fill").foregroundColor(.green)
            case .warning: Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.orange)
            case .off where dialog.kind == .mcp: Image(systemName: "circle").foregroundColor(.secondary)
            default: EmptyView()
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: row.label)
                    .font(row.state == .header ? .footnote.weight(.semibold) : .body)
                    .foregroundColor(row.state == .header ? .secondary : .primary)
                if !row.detail.isEmpty { Text(verbatim: row.detail).font(.caption).foregroundColor(.secondary) }
            }
            Spacer(minLength: 8)
            if let value = row.value {
                switch row.state {
                case .on: Image(systemName: "checkmark.circle.fill").foregroundColor(.green)
                case .off: Image(systemName: "xmark.circle").foregroundColor(.secondary)
                default: Text(verbatim: value).font(.callout).foregroundColor(.secondary).multilineTextAlignment(.trailing)
                }
            }
            if index == Int(dialog.selected), selectable { Image(systemName: "chevron.left").font(.caption2).foregroundColor(.appAccent) }
        }
        if selectable, row.state != .header {
            Button {
                var keys = DialogKeys.move(from: Int(dialog.selected), to: index)
                keys.append(dialog.kind == .config ? DialogKeys.enter : DialogKeys.enter)
                send(keys)
            } label: { content }.buttonStyle(.plain)
        } else {
            content
        }
    }
}

/// The agent's commands that open a dialog: one tap types the command and the dialog becomes a screen.
/// The entries come from the agent's row of the core's registry, so every agent gets its own list.
struct AgentCommandItems: View {
    let commands: [AgentCommand]
    let run: (String) -> Void

    var body: some View {
        ForEach(commands, id: \.name) { c in
            if c.dividerBefore { Divider() }
            Button(role: c.destructive ? .destructive : nil) { run(c.name) } label: {
                Label(c.label, systemImage: c.icon)
            }
        }
    }
}
