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

    private enum CodingKeys: String, CodingKey {
        case id, name, agentRaw, binary, env, args, workingDir, gitBranch, permissionMode, terminalSchemeId, loadShellRc
        case hookKindRaw, customHookCommand, customHookEvents, customHookMatcher
        case approvalMatcher, approvalEvents
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
        // Profiles saved before this option existed keep their previous (login-shell) behavior.
        loadShellRc = try c.decodeIfPresent(Bool.self, forKey: .loadShellRc) ?? false
        hookKindRaw = try c.decodeIfPresent(String.self, forKey: .hookKindRaw) ?? "phoneApproval"
        customHookCommand = try c.decodeIfPresent(String.self, forKey: .customHookCommand) ?? ""
        customHookEvents = try c.decodeIfPresent([String].self, forKey: .customHookEvents) ?? []
        customHookMatcher = try c.decodeIfPresent(String.self, forKey: .customHookMatcher) ?? ""
        approvalMatcher = try c.decodeIfPresent(String.self, forKey: .approvalMatcher) ?? ""
        approvalEvents = try c.decodeIfPresent([String].self, forKey: .approvalEvents) ?? []
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
        self.loadShellRc = loadShellRc
        self.hookKindRaw = "phoneApproval"
        self.customHookCommand = ""
        self.customHookEvents = []
        self.customHookMatcher = ""
        self.approvalMatcher = ""
        self.approvalEvents = []
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
        }
    }

    static func kind(_ s: String) -> AgentKind {
        switch s {
        case "claudeCode": return .claudeCode
        case "antigravity": return .antigravity
        case "codex": return .codex
        default: return .grok
        }
    }

    public init(_ p: AgentLaunchProfile) {
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
        }
    }

    public var supportsPermissions: Bool { agent == .claudeCode }

    public var supportsHooks: Bool { true }

    /// Compiled launch command, or the validation error from the core.
    public func launchCommand() -> Result<String, Error> {
        Result { try buildAgentLaunchCommand(profile: coreProfile) }
    }
}

@MainActor
public final class AgentProfilesManager: ObservableObject {
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
    }
}
