import SwiftUI
import SshCoreBridge

/// The settings root: pages grouped by meaning. Every page is its own view; long forms fold into
/// disclosure groups so a page shows what matters first.
public struct SettingsView: View {
    @ObservedObject public var serversManager: ServerProfilesManager
    @ObservedObject private var subs = SubscriptionManager.shared
    @AppStorage(VaultSync.enabledKey) private var syncEnabled = false
    @AppStorage(VaultSync.noticeKey) private var syncNotice = false
    @Environment(\.dismiss) private var dismiss

    /// The agent profile manager, supplied by the main screen (needs the live session).
    private let agentsPage: AnyView?
    private let hooksPage: AnyView?
    private let sandboxPage: AnyView?

    private let agentProfiles: AgentProfilesManager?

    public init(serversManager: ServerProfilesManager, agentProfiles: AgentProfilesManager? = nil, agentsPage: AnyView? = nil, hooksPage: AnyView? = nil, sandboxPage: AnyView? = nil) {
        self.serversManager = serversManager
        self.agentProfiles = agentProfiles
        self.agentsPage = agentsPage
        self.hooksPage = hooksPage
        self.sandboxPage = sandboxPage
    }

    private enum Page: Hashable { case servers, keys, terminal, appearance, agents, hooks, sandbox, subscription, modules, sync, security, backup }

    public var body: some View {
        NavigationStack {
            List {
                if Monetization.compiledIn {
                    Section {
                        NavigationLink(value: Page.subscription) {
                            HStack {
                                SettingsLabel("SSH-App Pro", "star.circle.fill", tint: .yellow)
                                Spacer()
                                Text(subs.paymentsEnabled ? (subs.isPro ? "Active" : "Free") : "Off").foregroundColor(.secondary)
                            }
                        }
                    }
                }
                Section("Connections") {
                    NavigationLink(value: Page.servers) { SettingsLabel("Servers", "server.rack", tint: .blue) }
                    NavigationLink(value: Page.keys) { SettingsLabel("SSH keys", "key.fill", tint: .orange) }
                }
                if agentsPage != nil {
                    Section("Agents") {
                        NavigationLink(value: Page.agents) { SettingsLabel("Agents", "play.rectangle.on.rectangle.fill", tint: .purple) }
                        if hooksPage != nil {
                            NavigationLink(value: Page.hooks) { SettingsLabel("Hooks", "hand.raised.fill", tint: .orange) }
                        }
                        if sandboxPage != nil {
                            NavigationLink(value: Page.sandbox) { SettingsLabel("Sandbox", "shield.lefthalf.filled", tint: .green) }
                        }
                    }
                }
                Section("Terminal & look") {
                    NavigationLink(value: Page.terminal) { SettingsLabel("Terminal", "terminal", tint: .gray) }
                    NavigationLink(value: Page.appearance) { SettingsLabel("Appearance", "paintpalette", tint: .pink) }
                }
                Section("Security") {
                    NavigationLink(value: Page.security) { SettingsLabel("App lock", "faceid", tint: .red) }
                }
                Section("Data") {
                    NavigationLink(value: Page.sync) {
                        HStack {
                            SettingsLabel("iCloud sync", "icloud", tint: .cyan)
                            Spacer()
                            Text(syncNotice ? "Turned off" : (syncEnabled ? "On" : "Off"))
                                .foregroundColor(syncNotice ? .orange : .secondary)
                        }
                    }
                    if agentProfiles != nil {
                        NavigationLink(value: Page.backup) { SettingsLabel("Backup", "externaldrive.badge.timemachine", tint: .indigo) }
                    }
                    NavigationLink(value: Page.modules) { SettingsLabel("Modules & backup", "slider.horizontal.3", tint: .green) }
                }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .navigationDestination(for: Page.self) { page in
                switch page {
                case .servers: ServersSettingsPage(serversManager: serversManager)
                case .keys: KeysSettingsPage(serversManager: serversManager)
                case .terminal: TerminalSettingsPage()
                case .appearance: AppearanceSettingsPage()
                case .agents: if let agentsPage { agentsPage }
                case .hooks: if let hooksPage { hooksPage }
                case .sandbox: if let sandboxPage { sandboxPage }
                case .subscription: PaywallView(embedded: true)
                case .modules: ModuleSettingsView()
                case .sync: SyncSettingsView()
                case .security: SecuritySettingsPage()
                case .backup: if let agentProfiles { BackupSettingsPage(serversManager: serversManager, agentsManager: agentProfiles) }
                }
            }
        }
    }
}

/// The module settings document (feature 46), kept as JSON in `UserDefaults`. Pieces of the app
/// that run off the main thread read it through `current()`.
@MainActor
final class AppConfigStore: ObservableObject {
    static let shared = AppConfigStore()
    nonisolated static let storageKey = "app_config_v1"

    @Published private(set) var config: AppConfigRoot

    private init() {
        config = AppConfigStore.current()
    }

    nonisolated static func current() -> AppConfigRoot {
        loadAppConfig(storedJson: UserDefaults.standard.string(forKey: storageKey) ?? "")
    }

    func replace(_ new: AppConfigRoot) {
        guard let json = try? saveAppConfig(config: new) else { return }
        UserDefaults.standard.set(json, forKey: Self.storageKey)
        config = new
    }

    func update(_ edit: (inout AppConfigRoot) -> Void) {
        var c = config
        edit(&c)
        replace(c)
    }
}

/// Docker and upload tuning, export, import and reset. Only the modules whose feature exists are here.
struct ModuleSettingsView: View {
    @ObservedObject private var store = AppConfigStore.shared
    @State private var showImporter = false
    @State private var notice: String?
    @State private var confirmResetAll = false

    var body: some View {
        Form {
            Section(header: Text("Docker"), footer: Text("How much of a container's log is fetched, and whether the container list refreshes itself while it is open.")) {
                Stepper("Log lines: \(store.config.docker.logTailLines)",
                        value: Binding(get: { Int(store.config.docker.logTailLines) },
                                       set: { v in store.update { $0.docker.logTailLines = UInt32(v) } }),
                        in: 50...1000, step: 50)
                Picker("Refresh the list", selection: Binding(get: { store.config.docker.autoRefreshSecs },
                                                              set: { v in store.update { $0.docker.autoRefreshSecs = v } })) {
                    Text("Only when I ask").tag(UInt32(0))
                    Text("Every 10 s").tag(UInt32(10))
                    Text("Every 30 s").tag(UInt32(30))
                    Text("Every 60 s").tag(UInt32(60))
                }
                Button("Reset Docker settings") { store.update { $0.docker = defaultAppConfig().docker } }
            }
            Section(header: Text("Pictures sent to the host"), footer: Text("Pictures are re-encoded on the phone before they leave it. Turning metadata removal off keeps location and device data in them. An imported file can never turn it off.")) {
                Picker("Longest side", selection: Binding(get: { store.config.uploads.maxDimension },
                                                          set: { v in store.update { $0.uploads.maxDimension = v } })) {
                    ForEach([UInt32(1024), 1536, 2048, 3072, 4096], id: \.self) { Text("\($0) px").tag($0) }
                }
                Stepper("JPEG quality: \(store.config.uploads.jpegQualityPercent)%",
                        value: Binding(get: { Int(store.config.uploads.jpegQualityPercent) },
                                       set: { v in store.update { $0.uploads.jpegQualityPercent = UInt32(v) } }),
                        in: 30...100, step: 5)
                Toggle("Remove location and device data", isOn: Binding(get: { store.config.uploads.stripMetadata },
                                                                        set: { v in store.update { $0.uploads.stripMetadata = v } }))
                Button("Reset picture settings") { store.update { $0.uploads = defaultAppConfig().uploads } }
            }
            Section(header: Text("Backup"), footer: Text(notice ?? "The file holds these settings only: no keys, servers or passwords. Its checksum detects damage, it is not a signature.")) {
                if let file = try? exportAppConfig(config: store.config) {
                    ShareLink(item: file, subject: Text("SSH Mobile settings")) { Label("Export settings", systemImage: "square.and.arrow.up") }
                }
                Button { showImporter = true } label: { Label("Import settings", systemImage: "square.and.arrow.down") }
                Button("Reset all settings", role: .destructive) { confirmResetAll = true }
            }
        }
        .navigationTitle("Modules & backup")
        .fileImporter(isPresented: $showImporter, allowedContentTypes: [.json, .plainText]) { result in
            guard case .success(let url) = result else { return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            do {
                let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                guard size <= 64 * 1024 else { throw SshCoreError.InvalidInput(reason: "the file is too large to be a settings file") }
                let text = try String(contentsOf: url, encoding: .utf8)
                let imported = try importAppConfig(current: store.config, fileJson: text)
                store.replace(imported.config)
                notice = imported.notes.isEmpty ? "Settings imported." : "Settings imported. " + imported.notes.joined(separator: " ")
            } catch {
                notice = "Not imported: \(error.localizedDescription)"
            }
        }
        .confirmationDialog("Reset all module settings?", isPresented: $confirmResetAll, titleVisibility: .visible) {
            Button("Reset all", role: .destructive) { store.replace(defaultAppConfig()); notice = "Settings reset." }
            Button("Cancel", role: .cancel) {}
        }
    }
}
