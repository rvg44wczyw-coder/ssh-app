import SwiftUI
import PhotosUI
import SshCoreBridge

/// Launch profiles (auto-start of Claude / Antigravity / ...) and approval-hook management.
public struct AgentProfilesSheetView: View {
    @ObservedObject var manager: AgentProfilesManager
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onLaunch: (SavedAgentProfile, String) -> Void
    public var onClose: (() -> Void)?
    /// Shown inside another navigation stack (Settings): no own stack, no Close button.
    private let embedded: Bool

    @State private var editing: SavedAgentProfile? = nil
    @State private var hookStatus: [String: AgentHooksStatus] = [:]
    @State private var previewing: SavedAgentProfile? = nil
    @State private var editingPermissions: SavedAgentProfile? = nil
    @State private var showingGuard = false
    @State private var message: String? = nil
    @State private var approvalStatus: ApprovalStatus? = nil
    @State private var setupBusy = false
    @State private var setupLog: [String] = []
    @AppStorage("setup_include_limits") private var setupIncludeLimits = true

    public init(
        manager: AgentProfilesManager,
        viewModel: TerminalSessionViewModel,
        onLaunch: @escaping (SavedAgentProfile, String) -> Void,
        onClose: (() -> Void)? = nil,
        embedded: Bool = false
    ) {
        self.embedded = embedded
        self.manager = manager
        self.viewModel = viewModel
        self.onLaunch = onLaunch
        self.onClose = onClose
    }

    private var connected: Bool { viewModel.state == .connected }

    /// Agents whose group is folded (all open until you fold one); kept across launches.
    @AppStorage("profile_groups_closed") private var closedGroups = ""

    private var profileGroups: [(kind: AgentKind, profiles: [SavedAgentProfile])] {
        AgentKind.groupOrder.compactMap { kind in
            let list = manager.profiles.filter { $0.agent == kind }
            return list.isEmpty ? nil : (kind, list)
        }
    }

    private func groupBinding(_ kind: AgentKind) -> Binding<Bool> {
        Binding(
            get: { !closedGroups.split(separator: ",").contains(Substring(kind.groupTitle)) },
            set: { open in
                var closed = Set(closedGroups.split(separator: ",").map(String.init))
                if open { closed.remove(kind.groupTitle) } else { closed.insert(kind.groupTitle) }
                closedGroups = closed.sorted().joined(separator: ",")
            }
        )
    }

    public var body: some View {
        if embedded {
            content
        } else {
            NavigationStack { content }
        }
    }

    private var content: some View {
            List {
                if let message {
                    Text(message).font(.caption).foregroundColor(.orange)
                }
                Section(
                    header: Text("Auto-start in new tabs"),
                    footer: Text("Profiles run an explicit `env VAR=… binary` command, not shell aliases (aliases from ~/.zshrc are not loaded by the login shell the app uses).")
                ) {
                    Picker("New tab starts", selection: $manager.autoStartProfileId) {
                        Text("Plain shell").tag(String?.none)
                        ForEach(manager.profiles) { p in
                            Text(p.name).tag(Optional(p.id))
                        }
                    }
                }
                Section(
                    header: Text("Phone approvals and hooks"),
                    footer: Text("One tap sets up what is missing for every profile below: the approval hooks in each agent's config on the host (and this device's key, trusted there for approval signatures only). Needs OpenSSH (ssh-keygen) on the host. Requests reach the app while it is open. \"Review & install\" on a profile shows exactly what is written.")
                ) {
                    approvalStatusRow
                    Toggle("Also show Claude plan limits", isOn: $setupIncludeLimits)
                        .font(.caption)
                    Button {
                        Task { await setUpEverything() }
                    } label: {
                        HStack {
                            if setupBusy { ProgressView().controlSize(.small) }
                            Label(setupBusy ? "Setting up…" : "Set up everything", systemImage: "wand.and.stars")
                        }
                    }
                    .disabled(!connected || setupBusy || manager.profiles.allSatisfy { !$0.supportsHooks })
                    ForEach(setupLog, id: \.self) { line in
                        Text(line).font(.caption)
                            .foregroundColor(line.hasPrefix("✗") ? .red : .secondary)
                    }
                }
                Section(
                    header: Text("Host guard"),
                    footer: Text("Rules and texts kept on the host and enforced by the hook before anything reaches your phone: block, always ask, allow, add context, continue when the agent stops.")
                ) {
                    Button { showingGuard = true } label: { Label("Edit host guard", systemImage: "shield.lefthalf.filled") }
                        .disabled(!connected)
                }
                Section(header: Text("Profiles")) {
                    // One folder per agent (Claude Code, Antigravity, Grok, ...) with its profiles inside.
                    ForEach(profileGroups, id: \.kind) { group in
                        DisclosureGroup(isExpanded: groupBinding(group.kind)) {
                            ForEach(group.profiles) { profile in
                                profileRow(profile)
                            }
                        } label: {
                            HStack(spacing: 8) {
                                ProfileIconView(ref: ProfileIcons.defaultSymbol(for: group.kind), size: 16)
                                    .foregroundColor(.appAccent)
                                Text(group.kind.groupTitle).font(.headline)
                                Spacer()
                                Text("\(group.profiles.count)").font(.subheadline).foregroundColor(.secondary)
                            }
                        }
                    }
                    Menu {
                        Button {
                            editing = SavedAgentProfile(name: "", agent: .claudeCode, binary: "claude")
                        } label: { Label("Agent profile", systemImage: "person.crop.rectangle") }
                        Section("Local model") {
                            ForEach(localAgentTools(), id: \.id) { tool in
                                Button {
                                    editing = SavedAgentProfile(localTool: tool.id)
                                } label: { Label(tool.name, systemImage: ProfileIcons.defaultSymbol(for: tool.agent)) }
                            }
                        }
                    } label: {
                        Label("Add profile", systemImage: "plus")
                    }
                }
            }
            .navigationTitle("Agent Profiles")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if !embedded {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Close") { onClose?() }
                    }
                }
            }
            .sheet(item: $editing) { profile in
                AgentProfileEditorView(profile: profile, checkEnvironment: connected ? { binary in
                    guard let handle = viewModel.sessionHandle else { return nil }
                    return try? await handle.checkAgentEnvironment(binary: binary)
                } : nil, listDirectory: connected ? { path, hidden in
                    guard let handle = viewModel.sessionHandle else { throw SshCoreError.NotConnected }
                    return try await handle.listRemoteDirectory(path: path, includeHidden: hidden, helperSha256: await viewModel.helperPin())
                } : nil, listModels: connected ? {
                    guard let handle = viewModel.sessionHandle,
                          let status = try? await handle.checkHostOllama() else { return [] }
                    return status.models
                } : nil) { saved in
                    manager.upsert(saved)
                    editing = nil
                } onCancel: {
                    editing = nil
                }
            }
            .sheet(item: $previewing) { profile in
                HookPreviewSheetView(
                    profile: profile,
                    handle: connected ? viewModel.sessionHandle : nil,
                    onChanged: { Task { await refreshAll() } },
                    onClose: { previewing = nil }
                )
            }
            .sheet(isPresented: $showingGuard) {
                HostGuardView(handle: connected ? viewModel.sessionHandle : nil)
            }
            .sheet(item: $editingPermissions) { profile in
                PermissionsEditorSheetView(
                    profile: profile,
                    handle: connected ? viewModel.sessionHandle : nil,
                    onClose: { editingPermissions = nil }
                )
            }
            .task { await refreshAll() }
    }

    @ViewBuilder
    private func profileRow(_ profile: SavedAgentProfile) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                ProfileIconView(ref: ProfileIcons.rowSymbol(profile), size: 16)
                    .foregroundColor(ProfileIcons.color(profile) ?? .secondary)
                Text(profile.name).font(.headline)
                Spacer()
                Button("Open tab") {
                    switch profile.launchCommand() {
                    case .success(let cmd): onLaunch(profile, cmd)
                    case .failure(let e): message = e.localizedDescription
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }
            if !profile.workingDir.isEmpty || !profile.gitBranch.isEmpty {
                Label(
                    [profile.workingDir, profile.gitBranch.isEmpty ? nil : "⎇ " + profile.gitBranch]
                        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "  "),
                    systemImage: "folder"
                )
                .font(.caption).foregroundColor(.secondary).lineLimit(1)
            }
            Text(([profile.binary] + profile.env).joined(separator: "  "))
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(.secondary)
                .lineLimit(2)
            if profile.supportsHooks {
                hooksRow(profile)
                driftNotes(profile)
            } else {
                Text("Approval hooks: not supported yet").font(.caption2).foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 2)
        .swipeActions {
            Button(role: .destructive) { manager.delete(profile) } label: { Label("Delete", systemImage: "trash") }
            Button { editing = profile } label: { Label("Edit", systemImage: "pencil") }.tint(.blue)
            Button { editing = manager.duplicate(profile) } label: { Label("Duplicate", systemImage: "plus.square.on.square") }.tint(.indigo)
        }
    }

    @ViewBuilder
    private var approvalStatusRow: some View {
        if !connected {
            Label("Connect to a host to check", systemImage: "wifi.slash").font(.caption).foregroundColor(.secondary)
        } else if let st = approvalStatus {
            if !st.sshKeygen {
                Label("ssh-keygen not found on the host: phone approvals are unavailable", systemImage: "xmark.octagon")
                    .font(.caption).foregroundColor(.red)
            } else if !st.allowedSigners {
                Label("Not set up yet: press Set up everything", systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundColor(.orange)
            } else {
                Label("Ready (\(st.pending) waiting)", systemImage: "checkmark.shield.fill")
                    .font(.caption).foregroundColor(.green)
            }
        } else {
            Label("Checking…", systemImage: "hourglass").font(.caption).foregroundColor(.secondary)
        }
    }

    /// The host's hooks no longer match the profile's plan (edited by hand, or by the agent).
    @ViewBuilder
    private func driftNotes(_ profile: SavedAgentProfile) -> some View {
        if let status = hookStatus[profile.id], status.hooksInstalled,
           let planned = try? planAgentHooks(agent: profile.agent, preset: profile.hookPreset, hookDir: nil) {
            let drift = hooksDrift(planned: planned, installed: status.installed)
            if !drift.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Label("Hooks on the host differ from this profile", systemImage: "exclamationmark.triangle.fill")
                        .font(.caption.weight(.semibold)).foregroundColor(.orange)
                    ForEach(drift, id: \.self) { Text($0).font(.caption2).foregroundColor(.secondary) }
                    Text("If you did not change them, something edited the agent's config. Review & install restores the plan.")
                        .font(.caption2).foregroundColor(.secondary)
                }
            }
        }
    }

    @ViewBuilder
    private func hooksRow(_ profile: SavedAgentProfile) -> some View {
        let status = hookStatus[profile.id]
        HStack {
            Image(systemName: status?.hooksInstalled == true ? "checkmark.shield.fill" : "shield")
                .foregroundColor(status?.hooksInstalled == true ? .green : .secondary)
            Text(status == nil ? "Hooks: unknown" : (status!.hooksInstalled ? "Hooks installed" : "Hooks not installed"))
                .font(.caption)
            if let status, status.hooksInstalled,
               let planned = try? planAgentHooks(agent: profile.agent, preset: profile.hookPreset, hookDir: nil),
               !hooksDrift(planned: planned, installed: status.installed).isEmpty {
                Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.orange)
            }
            Spacer()
            if profile.supportsPermissions {
                Button("Permissions") { editingPermissions = profile }
                    .font(.caption)
                    .buttonStyle(.borderless)
            }
            Button("Review & install") { previewing = profile }
                .font(.caption)
                .buttonStyle(.borderless)
        }
    }

    /// Installs the approval hooks of every profile that lacks them (or whose hooks drifted) and, for
    /// Claude, the plan-limits wrapper, one agent config at a time; the result is listed per config.
    private func setUpEverything() async {
        guard connected, let handle = viewModel.sessionHandle else { return }
        setupBusy = true
        setupLog = []
        defer { setupBusy = false }
        var seen = Set<String>()
        for p in manager.profiles where p.supportsHooks {
            let key = "\(p.agent)|\(p.configDir ?? "")"
            guard seen.insert(key).inserted else { continue }
            let label = p.agent.groupTitle + (p.configDir.map { " (\($0))" } ?? "")
            do {
                let planned = (try? planAgentHooks(agent: p.agent, preset: p.hookPreset, hookDir: nil)) ?? []
                let status = try? await handle.agentHooksStatus(agent: p.agent, configDir: p.configDir, hookDir: nil)
                let current = status.map { $0.hooksInstalled && hooksDrift(planned: planned, installed: $0.installed).isEmpty } ?? false
                if !current {
                    try await handle.installAgentHooksWithPreset(
                        agent: p.agent, configDir: p.configDir, hookDir: nil, preset: p.hookPreset)
                }
                setupLog.append("✓ \(label): hooks " + (current ? "already set up" : "installed"))
                if setupIncludeLimits, p.agent == .claudeCode,
                   (try? await handle.claudeStatuslineStatus(configDir: p.configDir)) != true {
                    try await handle.installClaudeStatusline(configDir: p.configDir, hookDir: nil)
                    setupLog.append("✓ \(label): plan limits wrapper installed")
                }
            } catch {
                setupLog.append("✗ \(label): \(error.localizedDescription)")
            }
        }
        if setupLog.isEmpty { setupLog = ["Nothing to set up."] }
        await refreshAll()
    }

    private func refreshAll() async {
        guard connected, let handle = viewModel.sessionHandle else { return }
        approvalStatus = try? await handle.approvalStatus()
        for p in manager.profiles where p.supportsHooks {
            if let s = try? await handle.agentHooksStatus(agent: p.agent, configDir: p.configDir, hookDir: nil) {
                hookStatus[p.id] = s
            }
        }
    }
}

struct AgentProfileEditorView: View {
    @State var profile: SavedAgentProfile
    @State private var envText: String = ""
    @State private var pickedPhoto: PhotosPickerItem? = nil
    @State private var argsText: String = ""
    @State private var configDir: String = ""
    @State private var envCheck: AgentEnvCheck? = nil
    @State private var checking = false
    /// Probes the connected host; `nil` when there is no live session.
    var checkEnvironment: ((String) async -> AgentEnvCheck?)?
    /// Lists a directory on the connected host (for the folder picker); `nil` without a session.
    var listDirectory: ((String?, Bool) async throws -> RemoteDirectoryListing)?
    /// Models Ollama lists on the connected host; `nil` without a session.
    var listModels: (() async -> [String])?
    @State private var hostModels: [String] = []
    @State private var loadingModels = false
    @State private var pickingStart = false
    @State private var pickingConfig = false
    var onSave: (SavedAgentProfile) -> Void
    var onCancel: () -> Void

    init(
        profile: SavedAgentProfile,
        checkEnvironment: ((String) async -> AgentEnvCheck?)? = nil,
        listDirectory: ((String?, Bool) async throws -> RemoteDirectoryListing)? = nil,
        listModels: (() async -> [String])? = nil,
        onSave: @escaping (SavedAgentProfile) -> Void,
        onCancel: @escaping () -> Void
    ) {
        _profile = State(initialValue: profile)
        self.checkEnvironment = checkEnvironment
        self.listDirectory = listDirectory
        self.listModels = listModels
        self.onSave = onSave
        self.onCancel = onCancel
    }

    private var configEnvName: String? { agentConfigDirEnv(agent: profile.agent) }

    private func iconCell(_ symbol: String, systemImage: String) -> some View {
        let selected = profile.iconSymbol == symbol
        let tint = AppAccent(rawValue: profile.iconColor)?.color ?? Color.appAccent
        return Button { profile.iconSymbol = symbol } label: {
            ProfileIconView(ref: systemImage, size: 17)
                .foregroundColor(selected ? .white : tint)
                .frame(width: 38, height: 38)
                .background(selected ? tint : Color.secondary.opacity(0.12))
                .cornerRadius(8)
        }
        .buttonStyle(.plain)
    }

    private func compiled() -> SavedAgentProfile {
        var p = profile
        var env = envText.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        if let key = configEnvName {
            env.removeAll { $0.hasPrefix(key + "=") }
            let dir = configDir.trimmingCharacters(in: .whitespaces)
            if !dir.isEmpty { env.insert("\(key)=\(dir)", at: 0) }
        }
        p.env = env
        p.args = argsText.split(separator: "\n").map(String.init).filter { !$0.isEmpty }
        return p
    }

    @ViewBuilder
    private func envCheckResult(_ r: AgentEnvCheck) -> some View {
        if let p = r.loginPath {
            Label("Found without rc: \(p)", systemImage: "checkmark.circle.fill")
                .font(.caption).foregroundColor(.green)
        } else if let p = r.interactivePath {
            Label("Found only after loading ~/.zshrc: \(p)", systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundColor(profile.loadShellRc ? .green : .orange)
            if !profile.loadShellRc {
                Text("Enable \"Load ~/.zshrc\" or set the binary to this full path.")
                    .font(.caption).foregroundColor(.orange)
            }
        } else if r.interactiveTimedOut {
            Label("Loading ~/.zshrc took longer than 10 s (it may be waiting for input). Use the binary's full path and turn the toggle off.", systemImage: "clock.badge.exclamationmark")
                .font(.caption).foregroundColor(.orange)
        } else {
            Label("\(profile.binary) was not found on the host (it may be an alias; use its real binary path).", systemImage: "xmark.octagon.fill")
                .font(.caption).foregroundColor(.red)
        }
    }

    private func looksLikeCredential(_ command: String) -> Bool {
        let c = command.lowercased()
        return ["bearer ", "token=", "password", "passwd", "secret", "apikey", "api_key", "sk-"].contains { c.contains($0) }
    }

    private var supportsHooks: Bool { profile.supportsHooks }

    private var localTool: LocalAgentTool? { localAgentTools().first { $0.id == profile.localTool } }

    /// Tool and Ollama model of a local-model profile. The command line is built by the core from
    /// these two fields, so nothing typed here reaches the shell unvalidated.
    @ViewBuilder
    private var localModelSection: some View {
        Section(header: Text("Local model"), footer: Text(localTool?.note ?? "")) {
            Picker("Tool", selection: Binding(
                get: { profile.localTool },
                set: { id in
                    profile.localTool = id
                    if let t = localAgentTools().first(where: { $0.id == id }) {
                        profile.agent = t.agent
                        if !t.takesModel { profile.localModel = "" }
                        if profile.name.isEmpty || localAgentTools().contains(where: { $0.name == profile.name }) { profile.name = t.name }
                    }
                }
            )) {
                ForEach(localAgentTools(), id: \.id) { Text($0.name).tag($0.id) }
            }
            if localTool?.takesModel == true {
                TextField("Model (empty = the tool or Ollama picks)", text: $profile.localModel)
                    .font(.system(.body, design: .monospaced))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                if let listModels {
                    Menu {
                        ForEach(hostModels, id: \.self) { m in
                            Button(m) { profile.localModel = m }
                        }
                        if hostModels.isEmpty && !loadingModels {
                            Text("No models found (is Ollama running?)")
                        }
                    } label: {
                        Label(loadingModels ? "Asking the host…" : "Choose from host", systemImage: "cube")
                    }
                    .onAppear { refreshModels(listModels) }
                } else {
                    Text("Connect to a host to pick from its models.").font(.caption).foregroundColor(.secondary)
                }
            }
            if let err = localModelError {
                Label(err, systemImage: "exclamationmark.triangle.fill").font(.caption).foregroundColor(.red)
            }
            if localTool?.hasApprovals == false {
                Label("No approval hook for this tool: it runs without the phone gate.", systemImage: "info.circle")
                    .font(.caption).foregroundColor(.secondary)
            }
        }
    }

    private var localModelError: String? {
        guard profile.isLocal else { return nil }
        do { _ = try buildLocalAgentProfile(tool: profile.localTool, model: profile.localModel) }
        catch { return error.localizedDescription }
        return nil
    }

    private func refreshModels(_ load: @escaping () async -> [String]) {
        guard !loadingModels else { return }
        loadingModels = true
        Task {
            hostModels = await load()
            loadingModels = false
        }
    }

    private var hookPresetError: String? {
        guard supportsHooks else { return nil }
        do { _ = try planAgentHooks(agent: profile.agent, preset: profile.hookPreset, hookDir: nil) }
        catch { return error.localizedDescription }
        return nil
    }

    private var validationError: String? {
        if let e = localModelError { return e }
        if let e = hookPresetError { return e }
        if profile.name.trimmingCharacters(in: .whitespaces).isEmpty { return "Name is required" }
        if case .failure(let e) = compiled().launchCommand() { return e.localizedDescription }
        return nil
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Profile") {
                    TextField("Name (e.g. Claude personal)", text: $profile.name)
                    if !profile.isLocal {
                        Picker("Agent", selection: Binding(get: { profile.agent }, set: { profile.agent = $0 })) {
                            Text("Claude Code").tag(AgentKind.claudeCode)
                            Text("Google Antigravity").tag(AgentKind.antigravity)
                            Text("OpenAI Codex").tag(AgentKind.codex)
                            Text("Grok").tag(AgentKind.grok)
                        }
                        TextField("Binary", text: $profile.binary)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                    }
                }
                if profile.isLocal { localModelSection }
                Section(
                    header: Text("Start directory"),
                    footer: Text("The agent starts here (e.g. ~/projects/app); empty = the host's home folder. Branch: `git switch` runs here first; if it fails the tab says so and the agent still starts. Make one profile per folder/branch (swipe → Duplicate).")
                ) {
                    TextField("~/projects/app", text: $profile.workingDir)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                    if listDirectory != nil {
                        Button { pickingStart = true } label: { Label("Choose folder on host", systemImage: "folder") }
                    }
                    TextField("Git branch (optional)", text: $profile.gitBranch)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                }
                if profile.agent == .claudeCode || profile.agent == .antigravity {
                    Section(
                        header: Text("Permission mode"),
                        footer: Text("The mode the agent starts in. Accept edits: file edits in the start folder go through without asking. Commands still follow the agent's rules (Claude: Permissions → template \"Work in the project\" adds a list of allowed commands). Antigravity has no command list we can write; its Auto mode runs commands without asking.")
                    ) {
                        Picker("Mode", selection: $profile.permissionMode) {
                            Text("Agent default").tag("")
                            if profile.agent == .claudeCode {
                                Text("Accept edits").tag("acceptEdits")
                                Text("Plan (read-only)").tag("plan")
                            } else {
                                Text("Accept edits").tag("accept-edits")
                                Text("Auto").tag("auto")
                                Text("Plan (read-only)").tag("plan")
                            }
                        }
                    }
                }
                Section(header: Text("Icon"), footer: Text("Shown on the tab and in this list. Default uses the agent's icon.")) {
                    Picker("Color", selection: $profile.iconColor) {
                        Text("Automatic").tag("")
                        ForEach(AppAccent.allCases) { Text($0.title).tag($0.rawValue) }
                    }
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), spacing: 10) {
                        iconCell("", systemImage: ProfileIcons.defaultSymbol(for: profile.agent))
                        ForEach(ProfileIcons.glyphs, id: \.self) { iconCell(ProfileIcons.glyphRef($0), systemImage: ProfileIcons.glyphRef($0)) }
                        ForEach(ProfileIcons.symbols, id: \.self) { iconCell($0, systemImage: $0) }
                        if profile.iconSymbol.hasPrefix("img:") {
                            iconCell(profile.iconSymbol, systemImage: profile.iconSymbol)
                        }
                    }
                    .padding(.vertical, 4)
                    PhotosPicker(selection: $pickedPhoto, matching: .images) {
                        Label(profile.iconSymbol.hasPrefix("img:") ? "Change my picture" : "Use my own picture", systemImage: "photo")
                    }
                    .onChange(of: pickedPhoto) { _, item in
                        guard let item else { return }
                        Task {
                            if let data = try? await item.loadTransferable(type: Data.self),
                               let ref = ProfileIcons.storeImage(data, replacing: profile.iconSymbol) {
                                profile.iconSymbol = ref
                            }
                            pickedPhoto = nil
                        }
                    }
                }
                Section(
                    header: Text("Terminal colors"),
                    footer: Text("Colors of tabs started from this profile. Default uses the scheme chosen for this agent in Settings → Appearance.")
                ) {
                    Picker("Scheme", selection: $profile.terminalSchemeId) {
                        Text("Agent default").tag("")
                        ForEach(TerminalScheme.all) { Text($0.name).tag($0.id) }
                    }
                }
                Section(
                    header: Text("Host shell environment"),
                    footer: Text("Tabs start through a login shell that does not read ~/.zshrc. Enable this to load it (PATH from nvm/npm, exports) before the agent starts.")
                ) {
                    Toggle("Load ~/.zshrc (interactive shell)", isOn: $profile.loadShellRc)
                    if let check = checkEnvironment {
                        Button {
                            Task {
                                checking = true
                                envCheck = await check(profile.binary)
                                checking = false
                            }
                        } label: {
                            HStack {
                                Text("Check on host")
                                if checking { Spacer(); ProgressView() }
                            }
                        }
                        .disabled(checking)
                        if let r = envCheck { envCheckResult(r) }
                    } else {
                        Text("Connect to a host to check where \(profile.binary) resolves.")
                            .font(.caption).foregroundColor(.secondary)
                    }
                }
                if let key = configEnvName {
                    Section(header: Text(key), footer: Text("Where this agent keeps its settings on the host, e.g. ~/.claude-personal. Empty = the agent's default (~/.claude). Hooks and Permissions are read from and written to this folder, and the agent is started with it. Restart the agent in open tabs after changing it.")) {
                        TextField("~/.claude", text: $configDir)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                        if listDirectory != nil {
                            Button { pickingConfig = true } label: { Label("Choose folder on host", systemImage: "folder") }
                        }
                    }
                }
                if supportsHooks {
                    Section(
                        header: Text("Approval hooks"),
                        footer: Text("Hooks are installed from the profile list (Review & install), where you see exactly what is written to the host.")
                    ) {
                        NavigationLink {
                            HookTemplatesView(agent: profile.agent) { t in profile.apply(template: t.preset) }
                        } label: {
                            Label("Templates (\(hookTemplatesFor(agent: profile.agent).count))", systemImage: "square.grid.3x3.square")
                        }
                        Picker("Preset", selection: $profile.hookKindRaw) {
                            Text("Phone approval").tag("phoneApproval")
                            Text("Notify only").tag("notifyOnly")
                            Text("Custom command").tag("custom")
                        }
                        if profile.hookKindRaw == "custom" {
                            TextField("Command run by the agent", text: $profile.customHookCommand, axis: .vertical)
                                .font(.system(size: 13, design: .monospaced))
                                .autocorrectionDisabled()
                                #if os(iOS)
                                .textInputAutocapitalization(.never)
                                #endif
                            if looksLikeCredential(profile.customHookCommand) {
                                Text("This command seems to contain a credential. It is stored unencrypted on this phone, written into the agent's config and visible in the host's process list. Read it from the host's environment instead.")
                                    .font(.caption).foregroundColor(.orange)
                            }
                            ForEach(lintCustomHookCommand(command: profile.customHookCommand), id: \.self) { w in
                                Label(w, systemImage: "exclamationmark.triangle")
                                    .font(.caption).foregroundColor(.orange)
                            }
                            ForEach(hookEventsFor(agent: profile.agent), id: \.self) { ev in
                                Toggle(ev, isOn: Binding(
                                    get: { profile.customHookEvents.contains(ev) },
                                    set: { on in
                                        profile.customHookEvents.removeAll { $0 == ev }
                                        if on { profile.customHookEvents.append(ev) }
                                    }))
                            }
                            TextField("Tool matcher (empty = default)", text: $profile.customHookMatcher)
                                .autocorrectionDisabled()
                                #if os(iOS)
                                .textInputAutocapitalization(.never)
                                #endif
                        }
                        if profile.hookKindRaw != "custom" {
                            ForEach(profile.approvalEventChoices, id: \.self) { ev in
                                Toggle(ev, isOn: Binding(
                                    get: { profile.approvalEvents.isEmpty || profile.approvalEvents.contains(ev) },
                                    set: { on in
                                        var current = profile.approvalEvents.isEmpty ? profile.approvalEventChoices : profile.approvalEvents
                                        current.removeAll { $0 == ev }
                                        if on { current.append(ev) }
                                        // All selected = the defaults (stored as empty).
                                        profile.approvalEvents = Set(current) == Set(profile.approvalEventChoices) ? [] : current
                                    }))
                            }
                            ForEach(profile.extraApprovalEvents, id: \.self) { ev in
                                Toggle("\(ev) (extra)", isOn: Binding(
                                    get: { profile.approvalEvents.contains(ev) },
                                    set: { on in
                                        var current = profile.approvalEvents.isEmpty ? profile.approvalEventChoices : profile.approvalEvents
                                        current.removeAll { $0 == ev }
                                        if on { current.append(ev) }
                                        profile.approvalEvents = Set(current) == Set(profile.approvalEventChoices) ? [] : current
                                    }))
                            }
                            if profile.hookKindRaw == "phoneApproval" {
                                TextField("Tools sent to the phone, e.g. Bash|Edit|Write (empty = all)", text: $profile.approvalMatcher)
                                    .autocorrectionDisabled()
                                    #if os(iOS)
                                    .textInputAutocapitalization(.never)
                                    #endif
                            }
                        }
                        if let e = hookPresetError {
                            Text(e).font(.caption).foregroundColor(.red)
                        }
                    }
                }
                Section(header: Text("Extra environment (KEY=VALUE per line)")) {
                    TextEditor(text: $envText).font(.system(size: 13, design: .monospaced)).frame(minHeight: 70)
                }
                Section(header: Text("Arguments (one per line)")) {
                    TextEditor(text: $argsText).font(.system(size: 13, design: .monospaced)).frame(minHeight: 50)
                }
                if compiled().args.contains(where: { $0.contains("dangerously-skip-permissions") || $0.contains("bypass") || $0.contains("yolo") }) {
                    Section {
                        Text("This flag disables permission prompts, so approval hooks will not be triggered and nothing is sent to your phone.")
                            .font(.caption).foregroundColor(.orange)
                    }
                }
                if let err = validationError {
                    Section { Text(err).font(.caption).foregroundColor(.red) }
                } else if case .success(let cmd) = compiled().launchCommand() {
                    Section("Command") {
                        Text(cmd).font(.system(size: 11, design: .monospaced))
                    }
                }
            }
            .navigationTitle("Agent Profile")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: onCancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { onSave(compiled()) }.disabled(validationError != nil)
                }
            }
            .sheet(isPresented: $pickingStart) {
                if let list = listDirectory {
                    RemoteDirectoryPickerView(title: "Start directory", showHidden: false, start: profile.workingDir, list: list) {
                        profile.workingDir = $0
                    }
                }
            }
            .sheet(isPresented: $pickingConfig) {
                if let list = listDirectory {
                    RemoteDirectoryPickerView(title: "Config directory", showHidden: true, start: configDir, list: list) {
                        configDir = $0
                    }
                }
            }
            .onChange(of: profile.agent) { _, agent in
                profile.permissionMode = ""  // the modes differ per agent
                // Events differ per agent; drop ones the new agent does not have (they would be
                // invisible in the toggle list yet still fail validation).
                let allowed = hookEventsFor(agent: agent)
                profile.customHookEvents.removeAll { !allowed.contains($0) }
                profile.approvalEvents.removeAll { !profile.approvalEventChoices.contains($0) }
            }
            .onChange(of: profile.hookKindRaw) { _, _ in
                profile.approvalEvents.removeAll { !profile.approvalEventChoices.contains($0) }
            }
            .onAppear {
                var env = profile.env
                if let key = configEnvName, let i = env.firstIndex(where: { $0.hasPrefix(key + "=") }) {
                    configDir = String(env[i].dropFirst(key.count + 1))
                    env.remove(at: i)
                }
                envText = env.joined(separator: "\n")
                argsText = profile.args.joined(separator: "\n")
            }
        }
        .interactiveDismissDisabled()
    }
}


extension AgentKind {
    /// Order of the agent folders in the profile list.
    static let groupOrder: [AgentKind] = [
        .claudeCode, .antigravity, .grok, .codex, .openCode, .qwenCode, .crush, .cline, .aider, .goose, .ollamaChat,
    ]

    var groupTitle: String {
        switch self {
        case .claudeCode: return "Claude Code"
        case .antigravity: return "Antigravity"
        case .codex: return "Codex"
        case .grok: return "Grok"
        case .openCode: return "OpenCode"
        case .qwenCode: return "Qwen Code"
        case .crush: return "Crush"
        case .cline: return "Cline"
        case .aider: return "Aider"
        case .goose: return "Goose"
        case .ollamaChat: return "Ollama"
        }
    }
}
