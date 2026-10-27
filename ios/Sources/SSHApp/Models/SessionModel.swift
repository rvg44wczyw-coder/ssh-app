import Foundation

public enum AgentType: String, CaseIterable, Identifiable, Codable, Sendable {
    case claudeCode = "Claude Code"
    case antigravity = "Google Antigravity"
    case localLlm = "Local LLM (Ollama)"
    case customShell = "Custom Shell"
    /// A shell inside a Docker container on the host (feature 45).
    case dockerContainer = "Docker container"

    public var id: String { rawValue }

    public var defaultSessionName: String {
        switch self {
        case .claudeCode: return "claude"
        case .antigravity: return "antigravity"
        case .localLlm: return "ollama"
        case .customShell: return "shell"
        case .dockerContainer: return "docker"
        }
    }

    public var iconName: String {
        switch self {
        case .claudeCode: return "brain"
        case .antigravity: return "sparkles"
        case .localLlm: return "cpu"
        case .customShell: return "terminal"
        case .dockerContainer: return "shippingbox.fill"
        }
    }

    public var defaultCommand: String? {
        switch self {
        case .claudeCode: return "claude"
        case .antigravity: return "agy"
        case .localLlm: return "ollama run llama3"
        case .customShell, .dockerContainer: return nil
        }
    }
}

/// How an agent tab is isolated from the server (spec 62). Plain tabs have neither mark.
public enum TabIsolation: String, Codable, Sendable {
    case sandbox
    case unprotected
}

public struct TabSession: Identifiable, Equatable, Sendable, Codable {
    public let id: UUID
    public var title: String
    public var agentType: AgentType
    public var sessionName: String
    public var command: String?
    public var serverId: UUID?
    /// The agent profile this tab was started from (permissions and config folder come from it).
    public var profileId: String?
    /// Colour scheme chosen for this one session (overrides the profile's and the agent's).
    public var schemeId: String?
    /// Folder the tab's tmux session starts in (a git worktree); beats the global terminal folder.
    public var startDir: String?
    /// Set for agent tabs started from the Sandbox screen; the tab strip shows it.
    public var isolation: TabIsolation?
    /// The sandbox this tab's agent runs in (`SandboxRecord.id`), for pushes and the Sandbox screen.
    public var sandboxId: String?

    public init(
        id: UUID = UUID(),
        title: String,
        agentType: AgentType = .customShell,
        sessionName: String? = nil,
        command: String? = nil,
        serverId: UUID? = nil,
        profileId: String? = nil,
        schemeId: String? = nil,
        startDir: String? = nil,
        isolation: TabIsolation? = nil,
        sandboxId: String? = nil
    ) {
        self.id = id
        self.title = title
        self.agentType = agentType
        self.sessionName = sessionName ?? "term-\(id.uuidString.prefix(6).lowercased())"
        self.command = command
        self.serverId = serverId
        self.profileId = profileId
        self.schemeId = schemeId
        self.startDir = startDir
        self.isolation = isolation
        self.sandboxId = sandboxId
    }

    public static func defaultSessions() -> [TabSession] {
        [
            TabSession(
                title: "Terminal 1",
                agentType: .customShell,
                sessionName: "term-1",
                command: nil
            )
        ]
    }
}
