import SwiftUI
import WebKit
import SshCoreBridge

/// The page a diagram or an HTML snippet is drawn in (feature 41). Pure text in, text out, so the
/// security properties can be tested: the Content Security Policy allows inline script and style
/// and nothing else, the agent's text never becomes markup of the Mermaid page (it travels as
/// base64), and an HTML snippet gets no script source at all.
enum ArtifactPage {
    static let mermaidCSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:"
    static let htmlCSP = "default-src 'none'; style-src 'unsafe-inline'; img-src data:"

    private static let head = """
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1, minimum-scale=0.2, maximum-scale=10, user-scalable=yes">
    """

    private static func css(dark: Bool) -> String {
        let (fg, bg) = dark ? ("#e6e6e6", "#000000") : ("#1c1c1e", "#ffffff")
        return "html,body{margin:0;padding:8px;background:\(bg);color:\(fg);font:15px -apple-system,sans-serif}svg{max-width:none;height:auto}"
    }

    static func mermaid(source: String, mermaidJS: String, dark: Bool) -> String {
        let b64 = Data(source.utf8).base64EncodedString()
        // A script tag cannot be closed from inside the library text.
        let lib = mermaidJS.replacingOccurrences(of: "</script", with: "<\\/script")
        return """
        <!doctype html><html><head>\(head)
        <meta http-equiv="Content-Security-Policy" content="\(mermaidCSP)">
        <style>\(css(dark: dark))</style></head>
        <body><div id="out"></div><div id="src" data-b64="\(b64)" hidden></div>
        <script>\(lib)</script>
        <script>
        (async () => {
          const post = (o) => window.webkit.messageHandlers.done.postMessage(o);
          try {
            const bytes = Uint8Array.from(atob(document.getElementById('src').dataset.b64), c => c.charCodeAt(0));
            const text = new TextDecoder().decode(bytes);
            mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: '\(dark ? "dark" : "default")' });
            const r = await mermaid.render('diagram', text);
            document.getElementById('out').innerHTML = r.svg;
            post({ ok: true, svg: r.svg });
          } catch (e) { post({ ok: false, error: String(e && e.message || e).slice(0, 300) }); }
        })();
        </script></body></html>
        """
    }

    /// The snippet is the body as written; with no script source in the policy and JavaScript
    /// switched off in the web view it cannot run anything.
    static func html(snippet: String, dark: Bool) -> String {
        """
        <!doctype html><html><head>\(head)
        <meta http-equiv="Content-Security-Policy" content="\(htmlCSP)">
        <style>\(css(dark: dark))</style></head><body>\(snippet)</body></html>
        """
    }

    /// The bundled library, or nil when it is missing or does not match the pinned hash.
    static func bundledMermaid() -> String? {
        guard let url = Bundle.main.url(forResource: "mermaid.min", withExtension: "js"),
              let data = try? Data(contentsOf: url),
              ArtifactPage.sha256(data) == pinnedMermaidSHA256 else { return nil }
        return String(data: data, encoding: .utf8)
    }

    static let pinnedMermaidSHA256 = "581ed7d74bd9048d0e3a91363927d72ef22942d7722546b27f7cc29e35390eb8"

    static func sha256(_ data: Data) -> String {
        var h = SHA256Hasher()
        h.update(data)
        return h.hexDigest()
    }
}

/// SHA-256 without importing CryptoKit into the file's public surface.
import CryptoKit
private struct SHA256Hasher {
    private var inner = SHA256()
    mutating func update(_ d: Data) { inner.update(data: d) }
    func hexDigest() -> String { inner.finalize().map { String(format: "%02x", $0) }.joined() }
}

/// Handle to the web view of the viewer: export reads from it.
@MainActor
final class ArtifactWebHandle: ObservableObject {
    weak var webView: WKWebView?
    @Published var svg: String?
    @Published var error: String?
    @Published var rendered = false

    func snapshotPNG() async -> Data? {
        guard let web = webView else { return nil }
        let cfg = WKSnapshotConfiguration()
        cfg.rect = CGRect(origin: .zero, size: web.scrollView.contentSize)
        guard let image = try? await web.takeSnapshot(configuration: cfg) else { return nil }
        return image.pngData()
    }
}

struct ArtifactWebView: UIViewRepresentable {
    let html: String
    let javaScript: Bool
    let handle: ArtifactWebHandle

    func makeCoordinator() -> Coordinator { Coordinator(handle: handle) }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let prefs = WKWebpagePreferences()
        prefs.allowsContentJavaScript = javaScript
        config.defaultWebpagePreferences = prefs
        config.userContentController.add(context.coordinator, name: "done")
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.isOpaque = false
        web.backgroundColor = .clear
        web.scrollView.backgroundColor = .clear
        handle.webView = web
        web.loadHTMLString(html, baseURL: nil)
        return web
    }

    func updateUIView(_ web: WKWebView, context: Context) {}

    static func dismantleUIView(_ web: WKWebView, coordinator: Coordinator) {
        web.configuration.userContentController.removeScriptMessageHandler(forName: "done")
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
        let handle: ArtifactWebHandle
        private var loaded = false
        init(handle: ArtifactWebHandle) { self.handle = handle }

        /// Only the first load is allowed: link taps, redirects and new windows open nothing.
        func webView(_ webView: WKWebView, decidePolicy action: WKNavigationAction) async -> WKNavigationActionPolicy {
            if !loaded, action.navigationType == .other, action.targetFrame?.isMainFrame ?? false {
                loaded = true
                return .allow
            }
            return .cancel
        }

        func userContentController(_ c: WKUserContentController, didReceive message: WKScriptMessage) {
            guard let body = message.body as? [String: Any] else { return }
            Task { @MainActor in
                if body["ok"] as? Bool == true {
                    handle.svg = body["svg"] as? String
                    handle.rendered = true
                } else {
                    handle.error = body["error"] as? String ?? "The diagram could not be drawn"
                }
            }
        }
    }
}

struct ArtifactViewerSheet: View {
    let artifact: ExtractedArtifact
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var handle = ArtifactWebHandle()
    @State private var confirmed = false
    @State private var exportURL: URL?
    @State private var copied = false

    private var dark: Bool { colorScheme == .dark }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                content
                Divider()
                actions.padding(10)
            }
            .navigationTitle(artifact.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Close") { dismiss() } }
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch artifact.kind {
        case .mermaid:
            if artifact.needsConfirmation && !confirmed {
                largeNotice
            } else if let js = ArtifactPage.bundledMermaid() {
                ZStack {
                    ArtifactWebView(html: ArtifactPage.mermaid(source: artifact.content, mermaidJS: js, dark: dark),
                                    javaScript: true, handle: handle)
                    if let e = handle.error {
                        Text(e).font(.footnote).foregroundStyle(.red).padding()
                    }
                }
            } else {
                Text("The diagram engine is missing or damaged.").foregroundStyle(.secondary).padding()
            }
        case .html:
            ArtifactWebView(html: ArtifactPage.html(snippet: artifact.content, dark: dark), javaScript: false, handle: handle)
        case .markdown:
            ReportView(blocks: ReportBlock.parse(artifact.content))
        }
    }

    private var largeNotice: some View {
        VStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle").font(.largeTitle).foregroundStyle(.orange)
            Text("Large diagram: \(artifact.lineCount) lines")
            Text("Drawing it can make the app slow.").font(.footnote).foregroundStyle(.secondary)
            Button("Draw anyway") { confirmed = true }.buttonStyle(.glassProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var actions: some View {
        HStack(spacing: 16) {
            Button {
                UIPasteboard.general.string = artifact.content
                copied = true
            } label: { Label(copied ? "Copied" : "Copy source", systemImage: "doc.on.doc") }
            Spacer()
            if artifact.kind == .mermaid {
                if let url = exportURL {
                    ShareLink(item: url) { Label("Share", systemImage: "square.and.arrow.up") }
                } else {
                    Menu {
                        Button("Export PNG") { Task { await export(png: true) } }
                        Button("Export SVG") { Task { await export(png: false) } }
                    } label: { Label("Export", systemImage: "square.and.arrow.up") }
                    .disabled(!handle.rendered)
                }
            }
        }
        .font(.subheadline)
    }

    private func export(png: Bool) async {
        let name = "diagram-\(artifact.id).\(png ? "png" : "svg")"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
        let data: Data? = png ? await handle.snapshotPNG() : handle.svg.map { Data($0.utf8) }
        guard let data, (try? data.write(to: url, options: .completeFileProtection)) != nil else { return }
        exportURL = url
    }
}

/// The diagrams and reports found in a feed, shown as a list that opens the viewer.
struct ArtifactListSheet: View {
    let artifacts: [ExtractedArtifact]
    @Environment(\.dismiss) private var dismiss
    @State private var opened: ExtractedArtifact?

    var body: some View {
        NavigationStack {
            List(artifacts, id: \.id) { a in
                Button { opened = a } label: {
                    HStack {
                        Image(systemName: icon(a.kind)).frame(width: 28)
                        VStack(alignment: .leading) {
                            Text(a.title)
                            Text("\(a.lineCount) lines").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("Diagrams and reports")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Close") { dismiss() } } }
            .sheet(item: Binding(get: { opened }, set: { opened = $0 })) { ArtifactViewerSheet(artifact: $0) }
        }
    }

    private func icon(_ k: ArtifactKind) -> String {
        switch k {
        case .mermaid: return "point.3.connected.trianglepath.dotted"
        case .markdown: return "doc.text"
        case .html: return "chevron.left.forwardslash.chevron.right"
        }
    }
}

extension ExtractedArtifact: Identifiable {}


/// A Markdown report as blocks the viewer draws natively (no web view, no HTML, no images, links are
/// never opened). Block structure only; inline styles are left to `AttributedString`.
enum ReportBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case bullet(text: String, depth: Int)
    case numbered(text: String, number: String, depth: Int)
    case quote(String)
    case code(String)
    case table([String])
    case rule

    /// At most this many blocks are drawn; the rest of a very long report is cut.
    static let maxBlocks = 2000

    static func parse(_ text: String) -> [ReportBlock] {
        var out: [ReportBlock] = []
        var paragraph: [String] = []
        var table: [String] = []
        var code: [String]?
        var fence = ""
        func emit(_ block: ReportBlock) {
            flush()
            out.append(block)
        }
        func flush() {
            if !paragraph.isEmpty { out.append(.paragraph(paragraph.joined(separator: " "))); paragraph = [] }
            if !table.isEmpty { out.append(.table(table)); table = [] }
        }
        for raw in text.components(separatedBy: "\n") {
            if out.count >= maxBlocks { break }
            let line = raw.replacingOccurrences(of: "\r", with: "")
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if code != nil {
                if trimmed.hasPrefix(fence) && trimmed.allSatisfy({ $0 == fence.first }) {
                    out.append(.code((code ?? []).joined(separator: "\n")))
                    code = nil
                } else {
                    code?.append(line)
                }
                continue
            }
            if let marker = ["```", "~~~"].first(where: { trimmed.hasPrefix($0) }) {
                flush()
                fence = String(trimmed.prefix(while: { $0 == marker.first }))
                code = []
                continue
            }
            if trimmed.isEmpty { flush(); continue }
            if trimmed.hasPrefix("|") { if !paragraph.isEmpty { flush() }; table.append(trimmed); continue }
            let hashes = trimmed.prefix(while: { $0 == "#" }).count
            if (1...6).contains(hashes), trimmed.dropFirst(hashes).first == " " {
                emit(.heading(level: hashes, text: trimmed.dropFirst(hashes).trimmingCharacters(in: .whitespaces)))
            } else if trimmed.count >= 3, trimmed.allSatisfy({ $0 == "-" }) || trimmed.allSatisfy({ $0 == "*" }) || trimmed.allSatisfy({ $0 == "_" }) {
                emit(.rule)
            } else if trimmed.hasPrefix("> ") || trimmed == ">" {
                emit(.quote(String(trimmed.dropFirst(1)).trimmingCharacters(in: .whitespaces)))
            } else if let bullet = ["- ", "* ", "+ "].first(where: { trimmed.hasPrefix($0) }) {
                let depth = (line.prefix(while: { $0 == " " || $0 == "\t" }).count) / 2
                emit(.bullet(text: String(trimmed.dropFirst(bullet.count)), depth: min(depth, 4)))
            } else if let dot = trimmed.firstIndex(where: { $0 == "." || $0 == ")" }),
                      dot != trimmed.startIndex, trimmed[..<dot].allSatisfy(\.isNumber), trimmed[..<dot].count <= 3,
                      trimmed[trimmed.index(after: dot)...].first == " " {
                let depth = (line.prefix(while: { $0 == " " || $0 == "\t" }).count) / 2
                emit(.numbered(text: String(trimmed[trimmed.index(after: dot)...]).trimmingCharacters(in: .whitespaces),
                                     number: String(trimmed[..<dot]), depth: min(depth, 4)))
            } else {
                if !table.isEmpty { out.append(.table(table)); table = [] }
                paragraph.append(trimmed)
            }
        }
        if let open = code { out.append(.code(open.joined(separator: "\n"))) }   // unclosed: shown as it is
        flush()
        return Array(out.prefix(maxBlocks))
    }
}

struct ReportView: View {
    let blocks: [ReportBlock]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in row(block) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
            .textSelection(.enabled)
        }
    }

    private func inline(_ s: String) -> Text { Text(FeedMarkdown.inline(s)) }

    @ViewBuilder private func row(_ block: ReportBlock) -> some View {
        switch block {
        case .heading(let level, let text):
            inline(text).font(level == 1 ? .title2.bold() : level == 2 ? .title3.bold() : .headline)
        case .paragraph(let text):
            inline(text)
        case .bullet(let text, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("\u{2022}")
                inline(text)
            }.padding(.leading, CGFloat(depth) * 16)
        case .numbered(let text, let number, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("\(number).").monospacedDigit()
                inline(text)
            }.padding(.leading, CGFloat(depth) * 16)
        case .quote(let text):
            inline(text).foregroundStyle(.secondary)
                .padding(.leading, 10)
                .overlay(alignment: .leading) { Rectangle().fill(Color.secondary.opacity(0.5)).frame(width: 3) }
        case .code(let code):
            ScrollView(.horizontal, showsIndicators: false) {
                Text(code).font(.system(size: 13, design: .monospaced)).padding(8)
            }
            .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
        case .table(let rows):
            ScrollView(.horizontal, showsIndicators: false) {
                Text(rows.joined(separator: "\n")).font(.system(size: 12, design: .monospaced))
            }
        case .rule:
            Divider()
        }
    }
}
