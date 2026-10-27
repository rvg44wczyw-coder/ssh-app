import SwiftUI
import SshCoreBridge

/// Starts an agent in a container with a copy of the project (spec 62): the agent sees only the
/// committed content, reaches only its own API hosts and asks the phone for approvals. Settings →
/// Agents → Sandbox. "Run without a sandbox" is always offered and is marked as unprotected.
struct SandboxView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @ObservedObject var profiles: AgentProfilesManager
    let onLaunch: (SavedAgentProfile, String, TabIsolation, String?) -> Void

    @State private var agent: SandboxAgent = .claudeCode
    @State private var folder = ""
    @State private var online = true
    @State private var autoApprove = false
    @State private var copyLogin = true
    @State private var engines: [SandboxEngine] = []
    @State private var engine: SandboxEngine = .docker
    @State private var detected = false
    @State private var copiedInstall = false
    /// The host and branch lists start folded (they can hold many entries).
    @State private var openAlways = false
    @State private var openSandboxes = true
    @State private var hostTarget: HostTarget? = nil
    @State private var busy: String? = nil
    @State private var error: String? = nil
    @State private var confirmUnprotected = false
    @State private var records: [SandboxRecord] = SandboxRegistry.all()
    @State private var alwaysHosts: [String] = []
    @State private var newAlways = ""

    private var connected: Bool { viewModel.state == .connected }
    private var normalizedFolder: String? { TerminalFolder.normalized(folder) }

    var body: some View {
        List {
            if !connected {
                Section { Label("Connect to a server to use a sandbox.", systemImage: "wifi.slash").foregroundColor(.secondary) }
            }
            if let error {
                Section { Text(error).font(.caption).foregroundColor(.red) }
            }
            Section {
                Picker("Agent", selection: $agent) {
                    Text("Claude Code").tag(SandboxAgent.claudeCode)
                    Text("Antigravity").tag(SandboxAgent.antigravity)
                    Text("Codex").tag(SandboxAgent.codex)
                    Text("Grok").tag(SandboxAgent.grok)
                }
                TextField("Project folder, e.g. ~/work/api", text: $folder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                if !savedFolders.isEmpty {
                    Picker("Known folders", selection: $folder) {
                        Text("—").tag("")
                        ForEach(savedFolders, id: \.self) { Text($0).tag($0) }
                    }
                }
                Toggle("Allow the agent’s own servers", isOn: $online)
                Toggle("Agent approves its own actions", isOn: $autoApprove)
                Toggle("Copy the agent’s login from the server", isOn: $copyLogin)
                if engines.count > 1 {
                    Picker("Container engine", selection: $engine) {
                        ForEach(engines, id: \.self) { Text($0 == .docker ? "Docker" : "Podman").tag($0) }
                    }
                }
            } header: {
                Text("Sandbox")
            } footer: {
                Text("The agent works on a copy of the committed files. Your home folder, SSH keys, other projects and Docker are not visible to it. With the switch off every approval comes to your phone; with it on the agent decides alone, but still inside the container. Offline: the container has no network. With the login copy on, the agent’s login file (and nothing else of its settings) is copied once from your account on the server, so the agent is signed in at once; code inside the sandbox can read that login. Running in a container is optional: “Run without a sandbox” starts the agent as before.")
            }

            Section {
                if !alwaysHosts.isEmpty {
                    DisclosureGroup("Hosts (\(alwaysHosts.count))", isExpanded: $openAlways) {
                        ForEach(alwaysHosts, id: \.self) { host in
                            Text(host).font(.footnote.monospaced())
                                .swipeActions {
                                    Button("Remove", role: .destructive) { removeAlways(host) }
                                }
                        }
                    }
                }
                HStack {
                    TextField("e.g. registry.npmjs.org or *.pypi.org", text: $newAlways)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Add") { addAlways(newAlways); newAlways = "" }
                        .disabled(newAlways.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } header: {
                Text("Always allowed for \(label)")
            } footer: {
                Text("Added to every new \(label) sandbox next to the agent’s own servers. A running sandbox keeps its own list; use “Allow always” on a blocked host to add it to both.")
            }

            Section {
                Button {
                    Task { await start() }
                } label: {
                    Label(busy ?? "Start in sandbox", systemImage: "shield.lefthalf.filled")
                }
                .disabled(!connected || busy != nil || normalizedFolder == nil || (detected && engines.isEmpty))
                Button(role: .destructive) {
                    confirmUnprotected = true
                } label: {
                    Label("Run without a sandbox (unprotected)", systemImage: "exclamationmark.shield")
                }
                .disabled(!connected || normalizedFolder == nil)
            } footer: {
                if detected && engines.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Neither Docker nor Podman was found on this server. Install one to get a sandbox, or run unprotected.")
                        if let hint = installHint {
                            Text(hint.text)
                            Button {
                                UIPasteboard.general.string = hint.command
                                copiedInstall = true
                            } label: {
                                Label(copiedInstall ? "Copied" : "Copy the install command", systemImage: copiedInstall ? "checkmark" : "doc.on.doc")
                            }
                            .buttonStyle(.bordered)
                            Button("Check again") { Task { await detect() } }
                                .buttonStyle(.bordered)
                        }
                    }
                }
            }

            let mine = SandboxRegistry.records(on: viewModel.session.serverId, in: records)
            if !mine.isEmpty {
                Section {
                    DisclosureGroup("Sandboxes (\(mine.count))", isExpanded: $openSandboxes) {
                        ForEach(mine) { r in
                            SandboxItemView(record: r, viewModel: viewModel, onChanged: reload, onAlways: { host in addAlways(host, agentName: r.agentName) })
                        }
                    }
                } header: {
                    Text("Running and kept")
                } footer: {
                    Text("Each sandbox is followed on its own. Branches arrive as sandbox/<id>/… in your project. Nothing is merged for you. Read the branch (especially hooks, CI files and build scripts) before you check it out: your machine runs those later.")
                }
            }
        }
        .navigationTitle("Sandbox")
        .task { await detect() }
        .onAppear { alwaysHosts = Self.loadAlways(agent) }
        .onChange(of: agent) { _, new in alwaysHosts = Self.loadAlways(new) }
        .confirmationDialog("Run without a sandbox?", isPresented: $confirmUnprotected, titleVisibility: .visible) {
            Button("Run unprotected", role: .destructive) { launchPlain() }
        } message: {
            Text("The agent runs with your user’s rights on the server and can read your keys and files. Only approvals from your phone stand in the way.")
        }
    }

    /// What to run on the server to get an engine. Nothing is run for you: you paste it in a
    /// terminal yourself, so an install never happens without you reading it.
    private var installHint: (text: String, command: String)? {
        switch hostTarget {
        case .darwinArm64, .darwinX8664:
            return ("On a Mac, paste this in a terminal on the Mac, then open Docker once so it starts.", "brew install --cask docker")
        case .linuxX8664, .linuxAarch64:
            return ("On Linux, install Podman with your package manager, for example on Debian or Ubuntu:", "sudo apt install podman")
        default:
            return nil
        }
    }

    private var savedFolders: [String] {
        Array(NSOrderedSet(array: profiles.profiles.filter { $0.isAvailable(on: viewModel.session.serverId) }.map(\.workingDir).compactMap { TerminalFolder.normalized($0) }).array.prefix(32)) as? [String] ?? []
    }

    private func profile(id: String) -> SandboxProfile? {
        guard let f = normalizedFolder else { return nil }
        return SandboxProfile(
            engine: engine, agent: agent, sandboxId: id, projectFolder: f,
            network: online ? .online : .offline, allowHosts: alwaysHosts,
            cpus: 2, memoryMb: 4096, pids: 512, autoApprove: autoApprove,
            baseImage: sandboxDefaultBaseImage(),
            copyLogin: copyLogin, claudeConfigDir: profiles.claudeConfigDirs.first ?? "")
    }

    private func detect() async {
        guard connected, let handle = viewModel.sessionHandle else { return }
        do {
            hostTarget = try? await handle.detectHostTarget()
            engines = try await handle.detectSandboxEngines()
            if let first = engines.first { engine = first }
            detected = true
        } catch {
            self.error = error.localizedDescription
        }
    }

    private static func agentName(_ agent: SandboxAgent) -> String {
        AgentResolver.entry(kind: sandboxKind(agent)).id
    }

    /// The agent kind a sandbox agent runs.
    static func sandboxKind(_ agent: SandboxAgent) -> AgentKind {
        switch agent {
        case .claudeCode: return .claudeCode
        case .antigravity: return .antigravity
        case .codex: return .codex
        case .grok: return .grok
        }
    }

    private static func loadAlways(_ agent: SandboxAgent) -> [String] {
        SandboxAllowStore.hosts(agentName(agent))
    }

    private func addAlways(_ host: String) {
        guard let h = SandboxAllowStore.normalized(host) else { error = "“\(host)” is not a valid host name."; return }
        guard !alwaysHosts.contains(h), alwaysHosts.count < SandboxAllowStore.maxPerAgent else { return }
        alwaysHosts.append(h)
        SandboxAllowStore.set(alwaysHosts, for: Self.agentName(agent))
        VaultSync.shared.sync()
    }

    /// "Allow always" from a sandbox of any agent, not only the one picked in the form.
    private func addAlways(_ host: String, agentName: String) {
        guard let h = SandboxAllowStore.normalized(host) else { return }
        var list = SandboxAllowStore.hosts(agentName)
        guard !list.contains(h), list.count < SandboxAllowStore.maxPerAgent else { return }
        list.append(h)
        SandboxAllowStore.set(list, for: agentName)
        if agentName == Self.agentName(agent) { alwaysHosts = list }
        VaultSync.shared.sync()
    }

    private func removeAlways(_ host: String) {
        alwaysHosts.removeAll { $0 == host }
        SandboxAllowStore.set(alwaysHosts, for: Self.agentName(agent))
        VaultSync.shared.sync()
    }

    private func newId() -> String {
        "sbx-" + String(UUID().uuidString.lowercased().replacingOccurrences(of: "-", with: "").prefix(8))
    }

    private func start() async {
        guard let handle = viewModel.sessionHandle else { return }
        let id = newId()
        guard let p = profile(id: id) else { return }
        busy = "Preparing… (the first time takes minutes)"
        defer { busy = nil }
        do {
            let command = try await handle.prepareSandbox(profile: p, hookPreset: defaultHookPreset())
            SandboxRegistry.upsert(SandboxRecord(
                id: id, agentName: Self.agentName(agent), folder: p.projectFolder,
                engineName: engine == .podman ? "podman" : "docker",
                online: online, autoApprove: autoApprove, copyLogin: copyLogin,
                claudeConfigDir: p.claudeConfigDir,
                serverId: viewModel.session.serverId?.uuidString, label: label, created: Date()))
            reload()
            error = nil
            let launch = SavedAgentProfile(name: "Sandbox " + label, agent: kind, binary: "sh", args: [], workingDir: "")
            onLaunch(launch, command, .sandbox, id)
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func launchPlain() {
        guard let f = normalizedFolder else { return }
        let bin = AgentResolver.entry(kind: kind).processNames.first ?? ""
        let p = SavedAgentProfile(name: "Unprotected " + label, agent: kind, binary: bin, workingDir: f)
        switch p.launchCommand() {
        case .success(let cmd): onLaunch(p, cmd, .unprotected, nil)
        case .failure(let e): error = e.localizedDescription
        }
    }

    /// Re-reads the list and tells the push extension which sandboxes exist.
    private func reload() {
        records = SandboxRegistry.all()
        PushManager.shared.republishNames()
    }

    private var label: String {
        switch agent {
        case .claudeCode: return "Claude Code"
        case .antigravity: return "Antigravity"
        case .codex: return "Codex"
        case .grok: return "Grok"
        }
    }

    private var kind: AgentKind { Self.sandboxKind(agent) }
}


/// One sandbox, followed on its own: what its proxy lets through and refuses, the branches it made,
/// and its removal. Polls only while the Sandbox screen is open.
private struct SandboxItemView: View {
    let record: SandboxRecord
    @ObservedObject var viewModel: TerminalSessionViewModel
    let onChanged: () -> Void
    let onAlways: (String) -> Void

    @State private var open = false
    @State private var openAllowed = false
    @State private var openBlocked = false
    @State private var openBranches = false
    @State private var allowed: [String] = []
    @State private var blocked: [String] = []
    @State private var reviews: [SandboxBranchReview] = []
    @State private var newHost = ""
    @State private var busy: String? = nil
    @State private var error: String? = nil
    @State private var confirmRemove = false

    private var connected: Bool { viewModel.state == .connected }
    private var profile: SandboxProfile { record.profile(allowHosts: SandboxAllowStore.hosts(record.agentName)) }

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            if let error { Text(error).font(.caption).foregroundColor(.red) }
            if record.online {
                if !allowed.isEmpty {
                    DisclosureGroup("Allowed (\(allowed.count))", isExpanded: $openAllowed) {
                        ForEach(allowed, id: \.self) { host in
                            Text(host).font(.footnote.monospaced())
                                .swipeActions { Button("Remove", role: .destructive) { Task { await disallow(host) } } }
                        }
                    }
                }
                HStack {
                    TextField("Add a host, e.g. *.pypi.org", text: $newHost)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Add") {
                        let h = newHost
                        newHost = ""
                        Task { await allow(h) }
                    }
                    .disabled(newHost.trimmingCharacters(in: .whitespaces).isEmpty || busy != nil)
                }
                if !blocked.isEmpty {
                    DisclosureGroup(isExpanded: $openBlocked) {
                        ForEach(blocked, id: \.self) { host in
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(host).font(.footnote.monospaced())
                                    Text("Blocked").font(.caption2).foregroundColor(.secondary)
                                }
                                Spacer()
                                Menu("Allow") {
                                    Button("This sandbox only") { Task { await allow(host) } }
                                    Button("Always for \(record.label)") {
                                        onAlways(host)
                                        Task { await allow(host) }
                                    }
                                }
                                .buttonStyle(.bordered).disabled(busy != nil)
                            }
                        }
                    } label: {
                        Text("Blocked (\(blocked.count))").foregroundStyle(.orange)
                    }
                }
            }
            Button { Task { await review() } } label: { Label(busy ?? "Bring back the agent’s branches", systemImage: "arrow.down.circle") }
                .disabled(!connected || busy != nil)
            if !reviews.isEmpty {
                DisclosureGroup("Branches (\(reviews.count))", isExpanded: $openBranches) {
                    ForEach(reviews, id: \.branch) { r in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(r.branch).font(.footnote.monospaced())
                            Text(r.changed == 1 ? "1 file changed" : "\(r.changed) files changed")
                                .font(.caption2).foregroundColor(.secondary)
                            if !r.flagged.isEmpty {
                                Label("Your machine may run these later. Read them first.", systemImage: "exclamationmark.triangle.fill")
                                    .font(.caption).foregroundStyle(.orange)
                                ForEach(r.flagged, id: \.self) { Text($0).font(.caption2.monospaced()) }
                            }
                        }
                    }
                }
            }
            Button(role: .destructive) { confirmRemove = true } label: { Label("Remove the sandbox", systemImage: "trash") }
                .disabled(!connected || busy != nil)
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text("\(record.label) · \(record.id)").font(.subheadline)
                    if !blocked.isEmpty { Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange) }
                }
                Text(record.folder).font(.caption2.monospaced()).foregroundColor(.secondary).lineLimit(1).truncationMode(.middle)
            }
        }
        .task(id: connected) {
            while !Task.isCancelled, connected, record.online {
                await refreshEgress()
                try? await Task.sleep(for: .seconds(8))
            }
        }
        .confirmationDialog("Remove this sandbox?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await remove() } }
        } message: {
            Text("Its container, network and work copy are deleted. Branches already brought back stay.")
        }
    }

    private func refreshEgress() async {
        guard let handle = viewModel.sessionHandle else { return }
        do {
            allowed = try await handle.sandboxAllowedHosts(profile: profile)
            blocked = try await handle.sandboxBlockedHosts(profile: profile)
        } catch {
            // The sandbox may be gone; removing it clears the entry.
        }
    }

    private func allow(_ host: String) async {
        guard let handle = viewModel.sessionHandle else { return }
        do {
            try await handle.allowSandboxHost(profile: profile, host: host)
            error = nil
            await refreshEgress()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func disallow(_ host: String) async {
        guard let handle = viewModel.sessionHandle else { return }
        do {
            try await handle.disallowSandboxHost(profile: profile, host: host)
            error = nil
            await refreshEgress()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func review() async {
        guard let handle = viewModel.sessionHandle else { return }
        busy = "Fetching…"
        defer { busy = nil }
        do {
            let branches = try await handle.fetchSandboxBranches(profile: profile)
            reviews = try await handle.reviewSandboxBranches(profile: profile, branches: branches)
                .sorted { ($0.flagged.isEmpty ? 1 : 0, $0.branch) < ($1.flagged.isEmpty ? 1 : 0, $1.branch) }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func remove() async {
        guard let handle = viewModel.sessionHandle else { return }
        busy = "Removing…"
        defer { busy = nil }
        do {
            try await handle.removeSandbox(profile: profile)
            SandboxRegistry.remove(record.id)
            onChanged()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
