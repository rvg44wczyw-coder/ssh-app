import SwiftUI
import WebKit
import SshCoreBridge

public struct ConsoleLogEntry: Identifiable {
    public let id = UUID()
    public let timestamp = Date()
    public let level: String
    public let message: String
}

public struct WebPreviewSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Environment(\.dismiss) private var dismiss

    @State private var selectedPort: UInt16 = 3000
    @State private var customPortText: String = ""
    @State private var forwardHandle: PortForwardHandle? = nil
    @State private var isLoading = false
    @State private var errorMessage: String? = nil
    @State private var consoleLogs: [ConsoleLogEntry] = []
    @State private var isConsoleExpanded = false
    @State private var webViewReloadTrigger = UUID()

    private let quickPorts: [UInt16] = [3000, 5173, 8000, 8080]

    public init(viewModel: TerminalSessionViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        NavigationStack {
            ZStack {
                Color(red: 0.07, green: 0.07, blue: 0.08)
                    .ignoresSafeArea()

                VStack(spacing: 0) {
                    portSelectorBar
                    dividerLine

                    if let handle = forwardHandle, handle.isActive() {
                        ZStack(alignment: .bottom) {
                            WebViewContainer(
                                urlString: handle.getLocalUrl(),
                                reloadTrigger: webViewReloadTrigger,
                                onConsoleLog: { level, msg in
                                    DispatchQueue.main.async {
                                        consoleLogs.append(ConsoleLogEntry(level: level, message: msg))
                                    }
                                }
                            )

                            if isConsoleExpanded {
                                consoleDrawer
                            }
                        }
                    } else if isLoading {
                        VStack(spacing: 12) {
                            ProgressView()
                                .tint(.accentColor)
                            Text("Opening SSH tunnel to port \(selectedPort)...")
                                .font(.system(size: 14, weight: .medium, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        emptyTunnelView
                    }

                    bottomStatusBar
                }
            }
            .navigationTitle("In-App Web Preview")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        stopTunnel()
                        dismiss()
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    if let handle = forwardHandle, handle.isActive() {
                        HStack(spacing: 12) {
                            Button {
                                webViewReloadTrigger = UUID()
                            } label: {
                                Image(systemName: "arrow.clockwise")
                            }

                            if let url = URL(string: handle.getLocalUrl()) {
                                ShareLink(item: url) {
                                    Image(systemName: "square.and.arrow.up")
                                }
                            }
                        }
                    }
                }
            }
        }
        .onAppear {
            if let active = viewModel.activePortForward, active.isActive() {
                self.forwardHandle = active
                self.selectedPort = active.getRemotePort()
            } else {
                startTunnel(port: selectedPort)
            }
        }
        .onDisappear {
            stopTunnel()
        }
    }

    private var portSelectorBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                Text("Remote Port:")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(.secondary)
                    .padding(.leading, 8)

                ForEach(quickPorts, id: \.self) { port in
                    Button {
                        selectedPort = port
                        startTunnel(port: port)
                    } label: {
                        Text(":\(port)")
                            .font(.system(size: 13, weight: .semibold, design: .monospaced))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(selectedPort == port ? Color.accentColor : Color(white: 0.16))
                            .foregroundColor(selectedPort == port ? .white : .primary)
                            .cornerRadius(8)
                    }
                }

                HStack(spacing: 4) {
                    TextField("Custom", text: $customPortText)
                        .keyboardType(.numberPad)
                        .font(.system(size: 13, design: .monospaced))
                        .frame(width: 60)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(Color(white: 0.14))
                        .cornerRadius(6)

                    Button("Go") {
                        if let port = UInt16(customPortText), port > 0 {
                            selectedPort = port
                            startTunnel(port: port)
                        }
                    }
                    .font(.system(size: 12, weight: .bold))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                    .background(Color.accentColor.opacity(0.8))
                    .foregroundColor(.white)
                    .cornerRadius(6)
                }
                .padding(.trailing, 8)
            }
            .padding(.vertical, 8)
        }
        .background(Color(red: 0.1, green: 0.1, blue: 0.12))
    }

    private var emptyTunnelView: some View {
        VStack(spacing: 16) {
            Image(systemName: "network")
                .font(.system(size: 48))
                .foregroundColor(.secondary)

            Text("No Active Port Forward")
                .font(.headline)

            if let error = errorMessage {
                Text(error)
                    .font(.caption)
                    .foregroundColor(.red)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal)
            }

            Button {
                startTunnel(port: selectedPort)
            } label: {
                Label("Start Forwarding :\(selectedPort)", systemImage: "play.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(Color.accentColor)
                    .foregroundColor(.white)
                    .cornerRadius(8)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var bottomStatusBar: some View {
        HStack {
            if let handle = forwardHandle, handle.isActive() {
                HStack(spacing: 6) {
                    Circle()
                        .fill(Color.green)
                        .frame(width: 8, height: 8)
                    Text("127.0.0.1:\(handle.getLocalPort()) ⇄ :\(handle.getRemotePort())")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundColor(.secondary)
                }
            } else {
                Text("Tunnel Stopped")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
            }

            Spacer()

            Button {
                isConsoleExpanded.toggle()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "terminal")
                    Text("Console (\(consoleLogs.count))")
                }
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(consoleLogs.isEmpty ? .secondary : .accentColor)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(Color(red: 0.1, green: 0.1, blue: 0.12))
    }

    private var consoleDrawer: some View {
        VStack(spacing: 0) {
            HStack {
                Text("DevTools Console")
                    .font(.caption.bold())
                Spacer()
                Button("Clear") {
                    consoleLogs.removeAll()
                }
                .font(.caption)
                Button {
                    isConsoleExpanded = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Color(white: 0.15))

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(consoleLogs) { log in
                        HStack(alignment: .top, spacing: 6) {
                            Text(log.level == "error" ? "🛑" : (log.level == "warn" ? "⚠️" : "💬"))
                                .font(.system(size: 10))
                            Text(log.message)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(log.level == "error" ? .red : .primary)
                        }
                    }
                }
                .padding(8)
            }
            .frame(height: 160)
            .background(Color(white: 0.08))
        }
        .cornerRadius(12, corners: [.topLeft, .topRight])
        .shadow(radius: 6)
    }

    private var dividerLine: some View {
        Rectangle()
            .fill(Color(white: 0.2))
            .frame(height: 0.5)
    }

    private func startTunnel(port: UInt16) {
        isLoading = true
        errorMessage = nil
        Task {
            do {
                let handle = try await viewModel.startPortForward(remotePort: port)
                await MainActor.run {
                    self.forwardHandle = handle
                    self.isLoading = false
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = error.localizedDescription
                    self.isLoading = false
                }
            }
        }
    }

    private func stopTunnel() {
        forwardHandle?.stop()
        forwardHandle = nil
        viewModel.stopPortForward()
    }
}

private struct WebViewContainer: UIViewRepresentable {
    let urlString: String
    let reloadTrigger: UUID
    let onConsoleLog: (String, String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onConsoleLog: onConsoleLog)
    }

    func makeUIView(context: Context) -> WKWebView {
        let contentController = WKUserContentController()
        contentController.add(context.coordinator, name: "consoleLog")

        // Injected JavaScript for console log capture
        let scriptSource = """
        (function() {
            var origLog = console.log;
            var origWarn = console.warn;
            var origErr = console.error;
            console.log = function() {
                var args = Array.from(arguments).map(String).join(' ');
                window.webkit.messageHandlers.consoleLog.postMessage({level: 'log', msg: args});
                origLog.apply(console, arguments);
            };
            console.warn = function() {
                var args = Array.from(arguments).map(String).join(' ');
                window.webkit.messageHandlers.consoleLog.postMessage({level: 'warn', msg: args});
                origWarn.apply(console, arguments);
            };
            console.error = function() {
                var args = Array.from(arguments).map(String).join(' ');
                window.webkit.messageHandlers.consoleLog.postMessage({level: 'error', msg: args});
                origErr.apply(console, arguments);
            };
        })();
        """
        let userScript = WKUserScript(source: scriptSource, injectionTime: .atDocumentStart, forMainFrameOnly: false)
        contentController.addUserScript(userScript)

        let config = WKWebViewConfiguration()
        config.userContentController = contentController

        let webView = WKWebView(frame: .zero, configuration: config)
        webView.backgroundColor = .clear
        webView.isOpaque = false

        if let url = URL(string: urlString) {
            webView.load(URLRequest(url: url))
        }

        return webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        if context.coordinator.lastReloadTrigger != reloadTrigger {
            context.coordinator.lastReloadTrigger = reloadTrigger
            if let url = URL(string: urlString) {
                uiView.load(URLRequest(url: url))
            }
        }
    }

    static func dismantleUIView(_ uiView: WKWebView, coordinator: Coordinator) {
        uiView.configuration.userContentController.removeScriptMessageHandler(forName: "consoleLog")
        uiView.stopLoading()
    }

    class Coordinator: NSObject, WKScriptMessageHandler {
        let onConsoleLog: (String, String) -> Void
        var lastReloadTrigger: UUID?

        init(onConsoleLog: @escaping (String, String) -> Void) {
            self.onConsoleLog = onConsoleLog
        }

        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            if message.name == "consoleLog", let dict = message.body as? [String: String] {
                let level = dict["level"] ?? "log"
                let msg = dict["msg"] ?? ""
                onConsoleLog(level, msg)
            }
        }
    }
}

extension View {
    func cornerRadius(_ radius: CGFloat, corners: UIRectCorner) -> some View {
        clipShape(RoundedCorner(radius: radius, corners: corners))
    }
}

private struct RoundedCorner: Shape {
    var radius: CGFloat = .infinity
    var corners: UIRectCorner = .allCorners

    func path(in rect: CGRect) -> Path {
        let path = UIBezierPath(
            roundedRect: rect,
            byRoundingCorners: corners,
            cornerRadii: CGSize(width: radius, height: radius)
        )
        return Path(path.cgPath)
    }
}
