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
            ScrollView {
                Text(markdown)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                    .textSelection(.enabled)
            }
        }
    }

    private var markdown: AttributedString {
        let opts = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: artifact.content, options: opts)) ?? AttributedString(artifact.content)
    }

    private var largeNotice: some View {
        VStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle").font(.largeTitle).foregroundStyle(.orange)
            Text("Large diagram: \(artifact.lineCount) lines")
            Text("Drawing it can make the app slow.").font(.footnote).foregroundStyle(.secondary)
            Button("Draw anyway") { confirmed = true }.buttonStyle(.borderedProminent)
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
