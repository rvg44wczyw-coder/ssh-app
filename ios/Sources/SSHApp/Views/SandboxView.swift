import SwiftUI
import SshCoreBridge

/// Starts an agent in a container with a copy of the project (spec 62): the agent sees only the
/// committed content, reaches only its own API hosts and asks the phone for approvals. Settings →
/// Agents → Sandbox. "Run without a sandbox" is always offered and is marked as unprotected.
struct SandboxView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @ObservedObject var profiles: AgentProfilesManager
    let onLaunch: (SavedAgentProfile, String) -> Void

    @State private var agent: SandboxAgent = .claudeCode
    @State private var folder = ""
    @State private var online = true
    @State private var autoApprove = false
    @State private var copyLogin = true
    @State private var engines: [SandboxEngine] = []
    @State private var engine: SandboxEngine = .docker
    @State private var detected = false
    @State private var busy: String? = nil
    @State private var error: String? = nil
    @State private var branches: [String] = []
    @State private var confirmUnprotected = false
    @State private var confirmRemove = false
    @State private var blocked: [String] = []
    @State private var allowed: [String] = []
    @State private var newHost = ""
    @State private var alwaysHosts: [String] = []
    @State private var newAlways = ""
    @AppStorage("sandbox.lastId") private var sandboxId = ""

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
                ForEach(alwaysHosts, id: \.self) { host in
                    Text(host).font(.footnote.monospaced())
                        .swipeActions {
                            Button("Remove", role: .destructive) { removeAlways(host) }
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
                    Text("Neither Docker nor Podman was found on this server. Install one (for example Docker Desktop on a Mac) to get a sandbox, or run unprotected.")
                }
            }

            if !sandboxId.isEmpty && online {
                Section {
                    ForEach(allowed, id: \.self) { host in
                        Text(host).font(.footnote.monospaced())
                            .swipeActions {
                                Button("Remove", role: .destructive) { Task { await disallow(host) } }
                            }
                    }
                    HStack {
                        TextField("Add a host, e.g. registry.npmjs.org or *.pypi.org", text: $newHost)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                        Button("Add") {
                            let h = newHost
                            newHost = ""
                            Task { await allow(h) }
                        }
                        .disabled(newHost.trimmingCharacters(in: .whitespaces).isEmpty || busy != nil)
                    }
                } header: {
                    Text("Reachable from the sandbox")
                } footer: {
                    Text("Swipe a host to remove it. Anything else is refused by the proxy; when the agent tries one it shows up below with Allow. Allow only what you recognise.")
                }
            }
            if !sandboxId.isEmpty {
                Section {
                    ForEach(blocked, id: \.self) { host in
                        HStack {
                            VStack(alignment: .leading) {
                                Text(host).font(.footnote.monospaced())
                                Text("Blocked").font(.caption2).foregroundColor(.secondary)
                            }
                            Spacer()
                            Menu("Allow") {
                                Button("This sandbox only") { Task { await allow(host) } }
                                Button("Always for \(label)") {
                                    addAlways(host)
                                    Task { await allow(host) }
                                }
                            }
                            .buttonStyle(.bordered).disabled(busy != nil)
                        }
                    }
                    Button { Task { await review() } } label: { Label("Bring back the agent’s branches", systemImage: "arrow.down.circle") }
                        .disabled(!connected || busy != nil || normalizedFolder == nil)
                    ForEach(branches, id: \.self) { Text($0).font(.footnote.monospaced()) }
                    Button(role: .destructive) { confirmRemove = true } label: { Label("Remove the sandbox", systemImage: "trash") }
                        .disabled(!connected || busy != nil || normalizedFolder == nil)
                } header: {
                    Text("Last sandbox")
                } footer: {
                    Text("Branches arrive as sandbox/\(sandboxId)/… in your project. Nothing is merged for you. Read the branch (especially hooks, CI files and build scripts) before you check it out: your machine runs those later.")
                }
            }
        }
        .navigationTitle("Sandbox")
        .task { await detect() }
        .onAppear { alwaysHosts = Self.loadAlways(agent) }
        .onChange(of: agent) { _, new in alwaysHosts = Self.loadAlways(new) }
        .task(id: sandboxId) {
            // While this screen is open and the sandbox is online, look for refused connections.
            while !Task.isCancelled, !sandboxId.isEmpty, online {
                await refreshEgress()
                try? await Task.sleep(for: .seconds(6))
            }
        }
        .confirmationDialog("Run without a sandbox?", isPresented: $confirmUnprotected, titleVisibility: .visible) {
            Button("Run unprotected", role: .destructive) { launchPlain() }
        } message: {
            Text("The agent runs with your user’s rights on the server and can read your keys and files. Only approvals from your phone stand in the way.")
        }
        .confirmationDialog("Remove the sandbox?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await remove() } }
        } message: {
            Text("Its container, network and work copy are deleted. Branches already brought back stay.")
        }
    }

    private var savedFolders: [String] {
        Array(NSOrderedSet(array: profiles.profiles.map(\.workingDir).compactMap { TerminalFolder.normalized($0) }).array.prefix(32)) as? [String] ?? []
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
            engines = try await handle.detectSandboxEngines()
            if let first = engines.first { engine = first }
            detected = true
        } catch {
            self.error = error.localizedDescription
        }
    }

    private static func agentName(_ agent: SandboxAgent) -> String {
        switch agent {
        case .claudeCode: return "claude"
        case .antigravity: return "agy"
        case .codex: return "codex"
        case .grok: return "grok"
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
            sandboxId = id
            error = nil
            let launch = SavedAgentProfile(name: "Sandbox " + label, agent: kind, binary: "sh", args: [], workingDir: "")
            onLaunch(launch, command)
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func launchPlain() {
        guard let f = normalizedFolder else { return }
        let bin: String
        switch agent {
        case .claudeCode: bin = "claude"
        case .antigravity: bin = "agy"
        case .codex: bin = "codex"
        case .grok: bin = "grok"
        }
        let p = SavedAgentProfile(name: "Unprotected " + label, agent: kind, binary: bin, workingDir: f)
        switch p.launchCommand() {
        case .success(let cmd): onLaunch(p, cmd)
        case .failure(let e): error = e.localizedDescription
        }
    }

    private func refreshEgress() async {
        guard connected, let handle = viewModel.sessionHandle, let p = profile(id: sandboxId) else { return }
        do {
            allowed = try await handle.sandboxAllowedHosts(profile: p)
            blocked = try await handle.sandboxBlockedHosts(profile: p)
        } catch {
            // The sandbox may be gone; the next removal or start resets the id.
        }
    }

    private func allow(_ host: String) async {
        guard let handle = viewModel.sessionHandle, let p = profile(id: sandboxId) else { return }
        do {
            try await handle.allowSandboxHost(profile: p, host: host)
            error = nil
            await refreshEgress()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func disallow(_ host: String) async {
        guard let handle = viewModel.sessionHandle, let p = profile(id: sandboxId) else { return }
        do {
            try await handle.disallowSandboxHost(profile: p, host: host)
            error = nil
            await refreshEgress()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func review() async {
        guard let handle = viewModel.sessionHandle, let p = profile(id: sandboxId) else { return }
        busy = "Fetching…"
        defer { busy = nil }
        do {
            branches = try await handle.fetchSandboxBranches(profile: p)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func remove() async {
        guard let handle = viewModel.sessionHandle, let p = profile(id: sandboxId) else { return }
        busy = "Removing…"
        defer { busy = nil }
        do {
            try await handle.removeSandbox(profile: p)
            sandboxId = ""
            branches = []
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private var label: String {
        switch agent {
        case .claudeCode: return "Claude Code"
        case .antigravity: return "Antigravity"
        case .codex: return "Codex"
        case .grok: return "Grok"
        }
    }

    private var kind: AgentKind {
        switch agent {
        case .claudeCode: return .claudeCode
        case .antigravity: return .antigravity
        case .codex: return .codex
        case .grok: return .grok
        }
    }
}
