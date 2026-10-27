import SwiftUI
import SshCoreBridge

/// Where the phone-approval hooks are active on the connected server (spec 61): per agent found on
/// that host, the user level ("All sessions") and every project the host or the phone knows.
/// Project is the normal choice; all sessions makes every session of the agent wait for the phone.
struct HooksOverviewView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @ObservedObject var profiles: AgentProfilesManager

    @State private var overview: [AgentHooksOverview] = []
    @State private var loading = false
    @State private var error: String? = nil
    @State private var showAll = false
    /// Agents whose project list is open; the lists start folded, only "All sessions" shows.
    @State private var openProjects: Set<String> = []
    @State private var busyKey: String? = nil
    @State private var confirmAll: AgentHooksOverview? = nil
    @State private var confirmReplace: PendingProject? = nil
    @State private var extraFolder = ""
    @State private var extraFolders: [String] = []
    @State private var categories: [String: String] = HookCategories.all()
    @State private var editingCategory: CategoryEdit? = nil
    @State private var categoryText = ""
    @State private var confirmBulk: BulkChange? = nil

    private struct CategoryEdit: Identifiable {
        let folder: String
        var id: String { folder }
    }

    /// "Enable / Remove for all in this category", shown before it runs.
    private struct BulkChange: Identifiable {
        let item: AgentHooksOverview
        let category: String
        let enable: Bool
        let targets: [HookTarget]
        var id: String { "\(item.agent)|\(category)|\(enable)" }
    }

    /// A project install waiting for the user to agree to switch "All sessions" off.
    private struct PendingProject: Identifiable {
        let item: AgentHooksOverview
        let scope: HookScope
        let key: String
        var id: String { key }
    }

    private var connected: Bool { viewModel.state == .connected }

    private func expanded(_ agent: String) -> Binding<Bool> {
        Binding(get: { openProjects.contains(agent) },
                set: { if $0 { openProjects.insert(agent) } else { openProjects.remove(agent) } })
    }

    var body: some View {
        List {
            if !connected {
                Section { Label("Connect to a server to see its hooks.", systemImage: "wifi.slash").foregroundColor(.secondary) }
            }
            if let error {
                Section { Text(error).font(.caption).foregroundColor(.red) }
            }
            ForEach(shown, id: \.agent) { item in
                Section {
                    row(item, target: item.user, title: "All sessions", scope: userScope(item), isUser: true)
                    if !item.projects.isEmpty {
                        DisclosureGroup(isExpanded: expanded(item.agent)) {
                            ForEach(groups(of: item), id: \.category) { group in
                                if showsCategoryHeaders(item) { categoryHeader(item, group.category, group.targets) }
                                ForEach(group.targets, id: \.folder) { p in
                                    row(item, target: p, title: folderTitle(p.folder), scope: projectScope(item, p), isUser: false)
                                        .contextMenu {
                                            if let f = p.folder {
                                                Button { categoryText = categories[categoryKey(f)] ?? ""; editingCategory = CategoryEdit(folder: f) } label: {
                                                    Label("Category…", systemImage: "tag")
                                                }
                                            }
                                        }
                                }
                            }
                        } label: {
                            Text("Projects (\(item.projects.count))")
                        }
                    }
                } header: {
                    Text(name(item.agent) + (item.found ? "" : " (not found)"))
                } footer: {
                    if let note = note(item.agent) { Text(note) }
                }
            }
            Section {
                Toggle("Show agents not found on this server", isOn: $showAll)
                HStack {
                    TextField("Other folder, e.g. ~/work/api", text: $extraFolder)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Add") { addFolder() }.disabled(TerminalFolder.normalized(extraFolder) == nil)
                }
            } footer: {
                Text("Hooks in a project only affect sessions started in that folder. “All sessions” makes every session of the agent wait for your phone, even ones not meant for it.")
            }
        }
        .navigationTitle("Hooks")
        .navigationBarTitleDisplayMode(.inline)
        .overlay { if loading && overview.isEmpty { ProgressView() } }
        .refreshable { await load() }
        .task { await load() }
        .alert("Category", isPresented: Binding(get: { editingCategory != nil }, set: { if !$0 { editingCategory = nil } })) {
            TextField("e.g. work", text: $categoryText)
            Button("Save") {
                if let e = editingCategory {
                    HookCategories.set(categoryText, server: serverId, folder: e.folder)
                    categories = HookCategories.all()
                }
                editingCategory = nil
            }
            Button("Cancel", role: .cancel) { editingCategory = nil }
        } message: {
            Text("A label for this folder. It only groups this list; it does not change where hooks are written. Leave it empty to remove it.")
        }
        .confirmationDialog(confirmBulk.map { ($0.enable ? "Enable" : "Remove") + " phone approval for \($0.targets.count) in “\($0.category)”?" } ?? "", isPresented: Binding(get: { confirmBulk != nil }, set: { if !$0 { confirmBulk = nil } }), titleVisibility: .visible, presenting: confirmBulk) { change in
            Button(change.enable ? "Enable" : "Remove", role: change.enable ? nil : .destructive) {
                confirmBulk = nil
                Task { await runBulk(change) }
            }
            Button("Cancel", role: .cancel) { confirmBulk = nil }
        } message: { change in
            Text(change.targets.compactMap(\.folder).map(folderTitle).joined(separator: "\n"))
        }
        .confirmationDialog("Turn off “All sessions”?", isPresented: Binding(get: { confirmReplace != nil }, set: { if !$0 { confirmReplace = nil } }), titleVisibility: .visible, presenting: confirmReplace) { pending in
            Button("Turn off All sessions and enable here", role: .destructive) {
                Task { await install(pending.item, scope: pending.scope, key: pending.key, disableUserLevel: true) }
                confirmReplace = nil
            }
            Button("Cancel", role: .cancel) { confirmReplace = nil }
        } message: { pending in
            Text("\(name(pending.item.agent)) now asks your phone in every session. Enabling it for this project switches that off: sessions in other folders will no longer wait for the phone until you enable them one by one.")
        }
        .confirmationDialog("Every session of this agent will wait for your phone", isPresented: Binding(get: { confirmAll != nil }, set: { if !$0 { confirmAll = nil } }), titleVisibility: .visible) {
            Button("Enable for all sessions", role: .destructive) {
                if let item = confirmAll { Task { await install(item, scope: userScope(item), key: "\(item.agent)|user") } }
                confirmAll = nil
            }
            Button("Cancel", role: .cancel) { confirmAll = nil }
        } message: {
            Text("Terminals that are not tied to the phone will then wait up to two minutes before the agent asks in its own terminal. Prefer enabling single projects.")
        }
    }

    private var serverId: UUID? { viewModel.session.serverId }

    private func categoryKey(_ folder: String) -> String { "\(serverId?.uuidString ?? "default")|\(folder)" }

    private func groups(of item: AgentHooksOverview) -> [(category: String, targets: [HookTarget])] {
        let byFolder = Dictionary(item.projects.compactMap { p in p.folder.map { ($0, p) } }, uniquingKeysWith: { a, _ in a })
        let order = item.projects.compactMap(\.folder)
        return HookCategories.grouped(order, server: serverId, in: categories).map { g in
            (g.category, g.folders.compactMap { byFolder[$0] })
        }
    }

    /// With no category set anywhere the list stays flat, as before.
    private func showsCategoryHeaders(_ item: AgentHooksOverview) -> Bool {
        item.projects.contains { p in p.folder.map { categories[categoryKey($0)] != nil } ?? false }
    }

    @ViewBuilder
    private func categoryHeader(_ item: AgentHooksOverview, _ category: String, _ targets: [HookTarget]) -> some View {
        let todo = targets.filter { $0.state != "ours" && $0.state != "unavailable" && $0.file != nil }
        let done = targets.filter { $0.state == "ours" }
        HStack {
            Label(category, systemImage: "tag").font(.caption.bold()).foregroundColor(.secondary)
            Spacer()
            if connected && (!todo.isEmpty || !done.isEmpty) {
                Menu {
                    if !todo.isEmpty {
                        Button("Enable for all in \(category)") { confirmBulk = BulkChange(item: item, category: category, enable: true, targets: todo) }
                    }
                    if !done.isEmpty {
                        Button("Remove for all in \(category)", role: .destructive) { confirmBulk = BulkChange(item: item, category: category, enable: false, targets: done) }
                    }
                } label: { Image(systemName: "ellipsis.circle") }
            }
        }
    }

    private func runBulk(_ change: BulkChange) async {
        if change.enable && change.item.user.state == "ours" {
            error = "“All sessions” is on for \(name(change.item.agent)). Turn it off first, then enable the projects."
            return
        }
        for t in change.targets {
            guard let scope = projectScope(change.item, t) else { continue }
            let key = "\(change.item.agent)|\(t.folder ?? "")"
            if change.enable { await install(change.item, scope: scope, key: key) } else { await remove(change.item, scope: scope, key: key) }
        }
    }

    private var shown: [AgentHooksOverview] { overview.filter { showAll || $0.found } }

    @ViewBuilder
    private func row(_ item: AgentHooksOverview, target: HookTarget, title: String, scope: HookScope?, isUser: Bool) -> some View {
        let key = "\(item.agent)|\(target.folder ?? "user")"
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(title).lineLimit(1).truncationMode(.middle)
                    if target.running { Text("running").font(.caption2).foregroundColor(.green) }
                }
                Text(stateText(target.state)).font(.caption).foregroundColor(stateColor(target.state))
            }
            Spacer()
            if busyKey == key {
                ProgressView()
            } else if let scope, connected, target.state != "unavailable" {
                if target.state == "ours" {
                    Button("Remove", role: .destructive) { Task { await remove(item, scope: scope, key: key) } }
                        .buttonStyle(.bordered)
                } else if isUser {
                    Button("Enable…") { confirmAll = item }.buttonStyle(.bordered)
                } else {
                    Button("Enable") {
                        if item.user.state == "ours" {
                            confirmReplace = PendingProject(item: item, scope: scope, key: key)
                        } else {
                            Task { await install(item, scope: scope, key: key) }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                }
            }
        }
    }

    private func kind(_ agent: String) -> AgentKind? {
        if let k = AgentResolver.all.first(where: { $0.id == agent })?.kind { return k }
        switch agent {
        case "claude": return .claudeCode
        case "agy": return .antigravity
        case "codex": return .codex
        case "grok": return .grok
        case "qwen": return .qwenCode
        case "crush": return .crush
        case "opencode": return .openCode
        default: return nil
        }
    }

    private func name(_ agent: String) -> String {
        if let d = AgentResolver.all.first(where: { $0.id == agent }) { return d.displayName }
        switch agent {
        case "claude": return "Claude Code"
        case "agy": return "Antigravity"
        case "codex": return "Codex"
        case "grok": return "Grok"
        case "qwen": return "Qwen Code"
        case "crush": return "Crush"
        case "opencode": return "OpenCode"
        default: return agent
        }
    }

    private func note(_ agent: String) -> String? {
        switch agent {
        case "codex": return "Codex runs only hooks you reviewed: after enabling, open /hooks in Codex and trust them. A project's hooks load only when Codex trusts that folder."
        case "grok": return "Project hooks need a one-time /hooks-trust in Grok. Grok hooks can only block, not approve."
        case "qwen": return "Restart Qwen Code in open tabs after changing hooks."
        case "crush": return "Project hooks are not available for Crush yet (its project file is not confirmed)."
        case "opencode": return "OpenCode loads the plugin when it starts: restart it in open tabs."
        default: return nil
        }
    }

    private var claudeDir: String? { profiles.claudeConfigDirs.first }

    private func userScope(_ item: AgentHooksOverview) -> HookScope? {
        kind(item.agent) == nil ? nil : .user(configDir: item.agent == "claude" ? claudeDir : nil)
    }

    private func projectScope(_ item: AgentHooksOverview, _ p: HookTarget) -> HookScope? {
        guard kind(item.agent) != nil, let folder = p.folder, p.file != nil else { return nil }
        return .project(folder: folder, userConfigDir: item.agent == "claude" ? claudeDir : nil)
    }

    private func folderTitle(_ folder: String?) -> String {
        guard let folder else { return "" }
        // /Users/<name>/… and /home/<name>/… read better as ~/…
        let parts = folder.split(separator: "/", omittingEmptySubsequences: true)
        if parts.count >= 2, parts[0] == "Users" || parts[0] == "home" {
            return parts.count == 2 ? "~" : "~/" + parts.dropFirst(2).joined(separator: "/")
        }
        return folder
    }

    private func stateText(_ state: String) -> String {
        switch state {
        case "ours": return "Phone approval is on"
        case "foreign": return "Other hooks only"
        case "unavailable": return "Not available for this agent yet"
        default: return "Not set"
        }
    }

    private func stateColor(_ state: String) -> Color { state == "ours" ? .green : .secondary }

    private func addFolder() {
        guard let f = TerminalFolder.normalized(extraFolder) else { return }
        if !extraFolders.contains(f) { extraFolders.append(f) }
        extraFolder = ""
        Task { await load() }
    }

    private func knownFolders() -> [String] {
        var folders = profiles.profiles.filter { $0.isAvailable(on: serverId) }.map(\.workingDir).compactMap { TerminalFolder.normalized($0) }
        folders.append(contentsOf: extraFolders)
        return Array(NSOrderedSet(array: folders).array.prefix(64)) as? [String] ?? []
    }

    private func load() async {
        guard connected, let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        loading = true
        defer { loading = false }
        do {
            overview = try await handle.agentHooksOverview(helperSha256: pin, folders: knownFolders(), claudeConfigDir: claudeDir)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func install(_ item: AgentHooksOverview, scope: HookScope?, key: String, disableUserLevel: Bool = false) async {
        guard let agent = kind(item.agent), let scope, let handle = viewModel.sessionHandle else { return }
        busyKey = key
        defer { busyKey = nil }
        do {
            try await handle.installAgentHooksWithPresetIn(agent: agent, scope: scope, hookDir: nil, preset: defaultHookPreset(), disableUserLevel: disableUserLevel)
            error = nil
        } catch {
            self.error = "Enable failed: " + error.localizedDescription
        }
        await load()
    }

    private func remove(_ item: AgentHooksOverview, scope: HookScope, key: String) async {
        guard let agent = kind(item.agent), let handle = viewModel.sessionHandle else { return }
        busyKey = key
        defer { busyKey = nil }
        do {
            try await handle.uninstallAgentHooksIn(agent: agent, scope: scope)
            error = nil
        } catch {
            self.error = "Remove failed: " + error.localizedDescription
        }
        await load()
    }
}
