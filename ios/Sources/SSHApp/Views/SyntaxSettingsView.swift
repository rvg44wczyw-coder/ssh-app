import SwiftUI

/// Settings → Syntax highlighting: on/off, colour theme, size limit and the highlighting modules.
struct SyntaxSettingsView: View {
    @AppStorage(SyntaxSettings.enabledKey) private var enabled = true
    @AppStorage(SyntaxSettings.themeKey) private var themeRaw = SyntaxTheme.auto.rawValue
    @AppStorage(SyntaxSettings.maxKBKey) private var maxKB = 200
    @Environment(\.colorScheme) private var scheme
    @State private var sample: [HighlightedLine]?
    @State private var counts: [String: Int] = [:]

    private static let sampleCode = """
    // Fibonacci, with a cache
    func fib(_ n: Int, _ memo: inout [Int: Int]) -> Int {
        if n < 2 { return n }
        if let hit = memo[n] { return hit }
        let value = fib(n - 1, &memo) + fib(n - 2, &memo)
        memo[n] = value
        return value
    }
    print("fib(30) = \\(fib(30, &cache))")
    """

    var body: some View {
        List {
            Section(footer: Text("Colours the code in the file preview. It runs on the phone, only for files of a known type and below the size limit; other files stay plain.")) {
                Toggle("Syntax highlighting", isOn: $enabled)
            }
            if enabled {
                Section(header: Text("Theme")) {
                    Picker("Theme", selection: $themeRaw) {
                        ForEach(SyntaxTheme.allCases) { Text($0.title).tag($0.rawValue) }
                    }
                    sampleView
                }
                Section(
                    header: Text("Size limit"),
                    footer: Text("Files larger than this are shown without colours.")
                ) {
                    Stepper("\(maxKB) KB", value: $maxKB, in: 10...2000, step: 50)
                }
                Section(
                    header: Text("Modules"),
                    footer: Text("Each module is an engine that colours a family of languages. Turn one off to stop using it; files it covers are then shown plain.")
                ) {
                    ForEach(Array(SyntaxService.modules.enumerated()), id: \.offset) { _, module in
                        ModuleRow(module: module, count: counts[module.id])
                    }
                }
            }
        }
        .navigationTitle("Syntax highlighting")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: "\(themeRaw)-\(scheme == .dark)") { await refreshSample() }
        .task {
            for module in SyntaxService.modules {
                counts[module.id] = await module.languageCount()
            }
        }
    }

    @ViewBuilder
    private var sampleView: some View {
        let lines = Self.sampleCode.components(separatedBy: "\n")
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(lines.enumerated()), id: \.offset) { idx, plain in
                Group {
                    if let sample, idx < sample.count {
                        Text(sample[idx].attributed())
                    } else {
                        Text(plain)
                    }
                }
                .font(.system(size: 11, design: .monospaced))
            }
        }
        .padding(.vertical, 4)
    }

    private func refreshSample() async {
        let theme = SyntaxTheme(rawValue: themeRaw) ?? .auto
        sample = await HighlightrModule.shared.highlight(
            Self.sampleCode, language: "swift", themeName: theme.names(dark: scheme == .dark))
    }
}

private struct ModuleRow: View {
    let module: any SyntaxModule
    let count: Int?
    @State private var on: Bool

    init(module: any SyntaxModule, count: Int?) {
        self.module = module
        self.count = count
        _on = State(initialValue: SyntaxSettings.isModuleEnabled(module.id))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Toggle(isOn: $on) {
                Text(module.title)
                if let count { Text("\(count) languages").font(.caption).foregroundColor(.secondary) }
            }
            .onChange(of: on) { _, v in
                UserDefaults.standard.set(v, forKey: SyntaxSettings.moduleKey(module.id))
            }
            Text(module.summary).font(.caption2).foregroundColor(.secondary)
        }
    }
}
