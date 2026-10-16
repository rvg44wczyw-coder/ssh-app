import SwiftUI
import SshCoreBridge

/// Launch profiles (auto-start of Claude / Antigravity / ...) and approval-hook management.
public struct AgentProfilesSheetView: View {
    @ObservedObject var manager: AgentProfilesManager
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onLaunch: (SavedAgentProfile, String) -> Void
    public var onClose: (() -> Void)?

    @State private var editing: SavedAgentProfile? = nil
    @State private var hookStatus: [String: AgentHooksStatus] = [:]
    @State private var previewing: SavedAgentProfile? = nil
    @State private var editingPermissions: SavedAgentProfile? = nil
    @State private var message: String? = nil
    @State private var approvalStatus: ApprovalStatus? = nil

    public init(
        manager: AgentProfilesManager,
        viewModel: TerminalSessionViewModel,
        onLaunch: @escaping (SavedAgentProfile, String) -> Void,
        onClose: (() -> Void)? = nil
    ) {
        self.manager = manager
        self.viewModel = viewModel
        self.onLaunch = onLaunch
        self.onClose = onClose
    }

    private var connected: Bool { viewModel.state == .connected }

    public var body: some View {
        NavigationStack {
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
                    header: Text("Phone approvals"),
                    footer: Text("Needs OpenSSH (ssh-keygen) on the host and this device's key trusted there. \"Review & install\" on a profile sets both up. Requests reach the app while it is open.")
                ) {
                    approvalStatusRow
                }
                Section(header: Text("Profiles")) {
                    ForEach(manager.profiles) { profile in
                        profileRow(profile)
                    }
                    Button {
                        editing = SavedAgentProfile(name: "", agent: .claudeCode, binary: "claude")
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
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { onClose?() }
                }
            }
            .sheet(item: $editing) { profile in
                AgentProfileEditorView(profile: profile, checkEnvironment: connected ? { binary in
                    guard let handle = viewModel.sessionHandle else { return nil }
                    return try? await handle.checkAgentEnvironment(binary: binary)
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
            .sheet(item: $editingPermissions) { profile in
                PermissionsEditorSheetView(
                    profile: profile,
                    handle: connected ? viewModel.sessionHandle : nil,
                    onClose: { editingPermissions = nil }
                )
            }
            .task { await refreshAll() }
        }
    }

    @ViewBuilder
    private func profileRow(_ profile: SavedAgentProfile) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
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
            Text(([profile.binary] + profile.env).joined(separator: "  "))
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(.secondary)
                .lineLimit(2)
            if profile.supportsHooks {
                hooksRow(profile)
            } else {
                Text("Approval hooks: not supported yet").font(.caption2).foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 2)
        .swipeActions {
            Button(role: .destructive) { manager.delete(profile) } label: { Label("Delete", systemImage: "trash") }
            Button { editing = profile } label: { Label("Edit", systemImage: "pencil") }.tint(.blue)
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
                Label("Not set up yet: press Review & install on a profile", systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundColor(.orange)
            } else {
                Label("Ready (\(st.pending) waiting)", systemImage: "checkmark.shield.fill")
                    .font(.caption).foregroundColor(.green)
            }
        } else {
            Label("Checking…", systemImage: "hourglass").font(.caption).foregroundColor(.secondary)
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
    @State private var argsText: String = ""
    @State private var configDir: String = ""
    @State private var envCheck: AgentEnvCheck? = nil
    @State private var checking = false
    /// Probes the connected host; `nil` when there is no live session.
    var checkEnvironment: ((String) async -> AgentEnvCheck?)?
    var onSave: (SavedAgentProfile) -> Void
    var onCancel: () -> Void

    init(
        profile: SavedAgentProfile,
        checkEnvironment: ((String) async -> AgentEnvCheck?)? = nil,
        onSave: @escaping (SavedAgentProfile) -> Void,
        onCancel: @escaping () -> Void
    ) {
        _profile = State(initialValue: profile)
        self.checkEnvironment = checkEnvironment
        self.onSave = onSave
        self.onCancel = onCancel
    }

    private var configEnvName: String? { agentConfigDirEnv(agent: profile.agent) }

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

    private var supportsHooks: Bool { profile.agent == .claudeCode || profile.agent == .antigravity }

    private var hookPresetError: String? {
        guard supportsHooks else { return nil }
        do { _ = try planAgentHooks(agent: profile.agent, preset: profile.hookPreset, hookDir: nil) }
        catch { return error.localizedDescription }
        return nil
    }

    private var validationError: String? {
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
                    Section(header: Text(key), footer: Text("Config directory on the host, e.g. ~/.claude-personal. Hooks are installed into this directory's settings.json.")) {
                        TextField("~/.claude", text: $configDir)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                    }
                }
                if supportsHooks {
                    Section(
                        header: Text("Approval hooks"),
                        footer: Text("Hooks are installed from the profile list (Review & install), where you see exactly what is written to the host.")
                    ) {
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
                Section("Working directory (optional)") {
                    TextField("~/projects/app", text: $profile.workingDir)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
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
            .onChange(of: profile.agent) { _, agent in
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
    }
}
