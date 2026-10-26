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
    @State private var busyKey: String? = nil
    @State private var confirmAll: AgentHooksOverview? = nil
    @State private var confirmReplace: PendingProject? = nil
    @State private var extraFolder = ""
    @State private var extraFolders: [String] = []

    /// A project install waiting for the user to agree to switch "All sessions" off.
    private struct PendingProject: Identifiable {
        let item: AgentHooksOverview
        let scope: HookScope
        let key: String
        var id: String { key }
    }

    private var connected: Bool { viewModel.state == .connected }

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
                    ForEach(item.projects, id: \.folder) { p in
                        row(item, target: p, title: folderTitle(p.folder), scope: projectScope(item, p), isUser: false)
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
        var folders = profiles.profiles.map(\.workingDir).compactMap { TerminalFolder.normalized($0) }
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
