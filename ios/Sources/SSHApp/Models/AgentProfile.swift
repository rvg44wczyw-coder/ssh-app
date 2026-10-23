import Foundation
import SshCoreBridge

/// Persisted form of a core `AgentLaunchProfile` (the bridge type is not `Codable`).
public struct SavedAgentProfile: Identifiable, Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var agentRaw: String
    public var binary: String
    /// Extra environment variables, `KEY=VALUE` per entry.
    public var env: [String]
    public var args: [String]
    public var workingDir: String
    /// Git branch switched to in `workingDir` before the agent starts (empty = leave as is).
    public var gitBranch: String
    /// Permission mode the agent starts in ("" = the agent's default): Claude `acceptEdits`, `plan`, ...; Antigravity `accept-edits`, `auto`, `plan`.
    public var permissionMode: String
    /// Terminal colour scheme id for tabs of this profile ("" = the agent's default scheme).
    public var terminalSchemeId: String
    /// Tab/profile icon (SF Symbol name, "" = by agent) and its color (`AppAccent` name, "" = automatic).
    public var iconSymbol: String
    public var iconColor: String
    /// Start through an interactive shell so `~/.zshrc` (PATH, exports) is loaded on the host.
    public var loadShellRc: Bool
    /// Approval-hook preset: "phoneApproval" (default), "notifyOnly" or "custom".
    public var hookKindRaw: String
    public var customHookCommand: String
    public var customHookEvents: [String]
    public var customHookMatcher: String
    /// Phone approval / notify only: tools that reach the phone (empty = all) and events to install (empty = all).
    public var approvalMatcher: String
    public var approvalEvents: [String]
    /// Local-model tool id ("opencode", "aider", ...; "" = an ordinary agent profile) and the Ollama
    /// model it is started with ("" = the tool or Ollama picks). For these profiles the launch
    /// binary/arguments/environment come from the core, so a model change is one edit.
    public var localTool: String
    public var localModel: String

    private enum CodingKeys: String, CodingKey {
        case id, name, agentRaw, binary, env, args, workingDir, gitBranch, permissionMode, terminalSchemeId, iconSymbol, iconColor, loadShellRc
        case hookKindRaw, customHookCommand, customHookEvents, customHookMatcher
        case approvalMatcher, approvalEvents
        case localTool, localModel
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        agentRaw = try c.decode(String.self, forKey: .agentRaw)
        binary = try c.decode(String.self, forKey: .binary)
        env = try c.decode([String].self, forKey: .env)
        args = try c.decode([String].self, forKey: .args)
        workingDir = try c.decode(String.self, forKey: .workingDir)
        gitBranch = try c.decodeIfPresent(String.self, forKey: .gitBranch) ?? ""
        permissionMode = try c.decodeIfPresent(String.self, forKey: .permissionMode) ?? ""
        terminalSchemeId = try c.decodeIfPresent(String.self, forKey: .terminalSchemeId) ?? ""
        iconSymbol = try c.decodeIfPresent(String.self, forKey: .iconSymbol) ?? ""
        iconColor = try c.decodeIfPresent(String.self, forKey: .iconColor) ?? ""
        // Profiles saved before this option existed keep their previous (login-shell) behavior.
        loadShellRc = try c.decodeIfPresent(Bool.self, forKey: .loadShellRc) ?? false
        hookKindRaw = try c.decodeIfPresent(String.self, forKey: .hookKindRaw) ?? "phoneApproval"
        customHookCommand = try c.decodeIfPresent(String.self, forKey: .customHookCommand) ?? ""
        customHookEvents = try c.decodeIfPresent([String].self, forKey: .customHookEvents) ?? []
        customHookMatcher = try c.decodeIfPresent(String.self, forKey: .customHookMatcher) ?? ""
        approvalMatcher = try c.decodeIfPresent(String.self, forKey: .approvalMatcher) ?? ""
        approvalEvents = try c.decodeIfPresent([String].self, forKey: .approvalEvents) ?? []
        localTool = try c.decodeIfPresent(String.self, forKey: .localTool) ?? ""
        localModel = try c.decodeIfPresent(String.self, forKey: .localModel) ?? ""
    }

    public init(
        id: String = UUID().uuidString,
        name: String,
        agent: AgentKind,
        binary: String,
        env: [String] = [],
        args: [String] = [],
        workingDir: String = "",
        gitBranch: String = "",
        permissionMode: String = "",
        terminalSchemeId: String = "",
        loadShellRc: Bool = true
    ) {
        self.id = id
        self.name = name
        self.agentRaw = Self.raw(agent)
        self.binary = binary
        self.env = env
        self.args = args
        self.workingDir = workingDir
        self.gitBranch = gitBranch
        self.permissionMode = permissionMode
        self.terminalSchemeId = terminalSchemeId
        self.iconSymbol = ""
        self.iconColor = ""
        self.loadShellRc = loadShellRc
        self.hookKindRaw = "phoneApproval"
        self.customHookCommand = ""
        self.customHookEvents = []
        self.customHookMatcher = ""
        self.approvalMatcher = ""
        self.approvalEvents = []
        self.localTool = ""
        self.localModel = ""
    }

    public var agent: AgentKind {
        get { Self.kind(agentRaw) }
        set { agentRaw = Self.raw(newValue) }
    }

    static func raw(_ k: AgentKind) -> String {
        switch k {
        case .claudeCode: return "claudeCode"
        case .antigravity: return "antigravity"
        case .codex: return "codex"
        case .grok: return "grok"
        case .openCode: return "openCode"
        case .qwenCode: return "qwenCode"
        case .crush: return "crush"
        case .cline: return "cline"
        case .aider: return "aider"
        case .goose: return "goose"
        case .ollamaChat: return "ollamaChat"
        }
    }

    static func kind(_ s: String) -> AgentKind {
        switch s {
        case "claudeCode": return .claudeCode
        case "antigravity": return .antigravity
        case "codex": return .codex
        case "openCode": return .openCode
        case "qwenCode": return .qwenCode
        case "crush": return .crush
        case "cline": return .cline
        case "aider": return .aider
        case "goose": return .goose
        case "ollamaChat": return .ollamaChat
        default: return .grok
        }
    }

    public init(_ p: AgentLaunchProfile) {
        let isLocal = p.id.hasPrefix("local-")
        self.init(
            id: p.id,
            name: p.name,
            agent: p.agent,
            binary: p.binary,
            env: p.env.map { "\($0.key)=\($0.value)" },
            args: p.args,
            workingDir: p.workingDir ?? "",
            gitBranch: p.gitBranch ?? "",
            permissionMode: p.permissionMode ?? "",
            loadShellRc: p.loadShellRc
        )
        if isLocal { localTool = String(p.id.dropFirst("local-".count)) }
    }

    /// Profile for a local-model tool (`localAgentTools()`), with an optional Ollama model.
    public init?(localTool tool: String, model: String = "") {
        guard let p = try? buildLocalAgentProfile(tool: tool, model: model) else { return nil }
        self.init(p)
        localModel = model
    }

    public var isLocal: Bool { !localTool.isEmpty }

    /// Whether ssh-app can install approval hooks for this profile's agent.
    public var supportsHooks: Bool {
        switch agent {
        case .cline, .aider, .goose, .ollamaChat: return false
        default: return true
        }
    }

    public var hookPreset: HookPreset {
        let kind: HookPresetKind
        switch hookKindRaw {
        case "notifyOnly": kind = .notifyOnly
        case "custom": kind = .custom
        default: kind = .phoneApproval
        }
        return HookPreset(
            kind: kind,
            customCommand: customHookCommand,
            customEvents: customHookEvents,
            customMatcher: customHookMatcher,
            approvalMatcher: approvalMatcher,
            approvalEvents: approvalEvents
        )
    }

    public var coreProfile: AgentLaunchProfile {
        // A local-model profile's command line is derived from tool + model by the core.
        if isLocal, let base = try? buildLocalAgentProfile(tool: localTool, model: localModel) {
            let dir = workingDir.trimmingCharacters(in: .whitespaces)
            let branch = gitBranch.trimmingCharacters(in: .whitespaces)
            let extra: [EnvVar] = env.compactMap { line in
                guard let eq = line.firstIndex(of: "=") else { return nil }
                return EnvVar(key: String(line[..<eq]), value: String(line[line.index(after: eq)...]))
            }.filter { e in !base.env.contains { $0.key == e.key } }
            return AgentLaunchProfile(
                id: id, name: name, agent: base.agent, binary: base.binary,
                env: base.env + extra, args: base.args + args.filter { !base.args.contains($0) },
                workingDir: dir.isEmpty ? nil : dir, gitBranch: branch.isEmpty ? nil : branch,
                permissionMode: nil, loadShellRc: loadShellRc
            )
        }
        let vars: [EnvVar] = env.compactMap { line in
            guard let eq = line.firstIndex(of: "=") else { return nil }
            return EnvVar(key: String(line[..<eq]), value: String(line[line.index(after: eq)...]))
        }
        let dir = workingDir.trimmingCharacters(in: .whitespaces)
        let branch = gitBranch.trimmingCharacters(in: .whitespaces)
        return AgentLaunchProfile(
            id: id,
            name: name,
            agent: agent,
            binary: binary,
            env: vars,
            args: args,
            workingDir: dir.isEmpty ? nil : dir,
            gitBranch: branch.isEmpty ? nil : branch,
            permissionMode: permissionMode.isEmpty ? nil : permissionMode,
            loadShellRc: loadShellRc
        )
    }

    /// Config dir the hooks must be installed into for this profile (`CLAUDE_CONFIG_DIR`), if any.
    public var configDir: String? { profileConfigDir(profile: coreProfile) }

    /// Events the phone-approval / notify-only presets install for this agent (the toggle list).
    public var approvalEventChoices: [String] {
        let notifyOnly = hookKindRaw == "notifyOnly"
        switch agent {
        case .claudeCode: return notifyOnly ? ["Notification", "Stop"] : ["PermissionRequest", "Notification", "Stop"]
        case .codex: return notifyOnly ? ["Stop"] : ["PermissionRequest", "Stop"]
        case .grok: return notifyOnly ? ["Notification", "Stop"] : ["PreToolUse", "Notification", "Stop"]
        case .antigravity: return notifyOnly ? ["Stop"] : ["PreToolUse", "Stop"]
        case .qwenCode: return notifyOnly ? ["Notification", "Stop"] : ["PermissionRequest", "Notification", "Stop"]
        case .openCode: return notifyOnly ? ["Stop"] : ["PreToolUse", "Stop"]
        case .crush: return notifyOnly ? [] : ["PreToolUse"]
        case .cline, .aider, .goose, .ollamaChat: return []
        }
    }

    /// Events the script-based presets can also install when chosen (recording and context
    /// events); never part of the default set, so they are off until switched on.
    public var extraApprovalEvents: [String] {
        guard agent == .claudeCode else { return [] }
        var extras = ["SessionStart", "SessionEnd", "UserPromptSubmit"]
        if hookKindRaw == "phoneApproval" { extras.append("PreToolUse") }
        return extras
    }

    /// Takes over the hook settings of a template.
    public mutating func apply(template preset: HookPreset) {
        switch preset.kind {
        case .phoneApproval: hookKindRaw = "phoneApproval"
        case .notifyOnly: hookKindRaw = "notifyOnly"
        case .custom: hookKindRaw = "custom"
        }
        customHookCommand = preset.customCommand
        customHookEvents = preset.customEvents
        customHookMatcher = preset.customMatcher
        approvalMatcher = preset.approvalMatcher
        approvalEvents = preset.approvalEvents
    }

    public var supportsPermissions: Bool { agent == .claudeCode }

    /// Compiled launch command, or the validation error from the core.
    public func launchCommand() -> Result<String, Error> {
        // A bad model name must fail the launch, not fall back to the plain profile in `coreProfile`.
        Result {
            if isLocal { _ = try buildLocalAgentProfile(tool: localTool, model: localModel) }
            return try buildAgentLaunchCommand(profile: coreProfile)
        }
    }
}

@MainActor
public final class AgentProfilesManager: ObservableObject {
    /// Config directories of every Claude profile, for the usage probe. A profile without one uses
    /// the default `~/.claude`, which is then scanned too.
    public var claudeConfigDirs: [String] {
        let claude = profiles.filter { $0.agent == .claudeCode }
        var dirs = claude.compactMap { $0.configDir }
        if claude.contains(where: { $0.configDir == nil }) && !dirs.isEmpty { dirs.append("~/.claude") }
        return Array(NSOrderedSet(array: dirs)) as? [String] ?? dirs
    }

    private static let profilesKey = "saved_agent_profiles"
    private static let autoStartKey = "agent_autostart_profile_id"

    @Published public var profiles: [SavedAgentProfile] = []
    /// Profile launched automatically in every new tab; `nil` means a plain shell.
    @Published public var autoStartProfileId: String? {
        didSet { UserDefaults.standard.set(autoStartProfileId, forKey: Self.autoStartKey) }
    }

    public init() {
        autoStartProfileId = UserDefaults.standard.string(forKey: Self.autoStartKey)
        if let data = UserDefaults.standard.data(forKey: Self.profilesKey),
           let loaded = try? JSONDecoder().decode([SavedAgentProfile].self, from: data),
           !loaded.isEmpty {
            profiles = loaded
        } else {
            profiles = defaultLaunchProfiles().map(SavedAgentProfile.init)
        }
        seedLocalPresets()
    }

    /// Adds the local-model tool presets once to profile lists saved before they existed. Deleted
    /// presets stay deleted: the flag is set after the first run.
    private func seedLocalPresets() {
        let key = "local_presets_seeded_v1"
        guard !UserDefaults.standard.bool(forKey: key) else { return }
        UserDefaults.standard.set(true, forKey: key)
        for tool in localAgentTools() where !profiles.contains(where: { $0.id == "local-\(tool.id)" }) {
            if let p = SavedAgentProfile(localTool: tool.id) { profiles.append(p) }
        }
        save()
    }

    /// One-time "this agent runs without the phone gate" confirmation, per profile.
    private static let ackKey = "local_agent_acknowledged"

    public func isAcknowledged(_ id: String) -> Bool {
        (UserDefaults.standard.stringArray(forKey: Self.ackKey) ?? []).contains(id)
    }

    public func acknowledge(_ id: String) {
        var ids = UserDefaults.standard.stringArray(forKey: Self.ackKey) ?? []
        if !ids.contains(id) { ids.append(id) }
        UserDefaults.standard.set(ids, forKey: Self.ackKey)
    }

    public var autoStartProfile: SavedAgentProfile? {
        guard let id = autoStartProfileId else { return nil }
        return profiles.first(where: { $0.id == id })
    }

    public func upsert(_ profile: SavedAgentProfile) {
        if let i = profiles.firstIndex(where: { $0.id == profile.id }) {
            profiles[i] = profile
        } else {
            profiles.append(profile)
        }
        save()
    }

    /// A copy of `profile` to adjust (another folder or config dir for the same agent).
    @discardableResult
    public func duplicate(_ profile: SavedAgentProfile) -> SavedAgentProfile {
        var copy = profile
        copy.id = UUID().uuidString
        copy.name = profile.name + " copy"
        upsert(copy)
        return copy
    }

    public func delete(_ profile: SavedAgentProfile) {
        profiles.removeAll { $0.id == profile.id }
        if autoStartProfileId == profile.id { autoStartProfileId = nil }
        save()
    }

    private func save() {
        if let data = try? JSONEncoder().encode(profiles) {
            UserDefaults.standard.set(data, forKey: Self.profilesKey)
        }
        Task { @MainActor in VaultSync.shared.sync() }
    }

    /// The list after an iCloud sync (feature 43): replaces the profiles without syncing again.
    func applySynced(_ list: [SavedAgentProfile]) {
        profiles = list
        if let id = autoStartProfileId, !list.contains(where: { $0.id == id }) { autoStartProfileId = nil }
        if let data = try? JSONEncoder().encode(list) {
            UserDefaults.standard.set(data, forKey: Self.profilesKey)
        }
    }
}
