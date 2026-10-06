import Foundation

public enum AgentType: String, CaseIterable, Identifiable, Codable, Sendable {
    case claudeCode = "Claude Code"
    case antigravity = "Google Antigravity"
    case localLlm = "Local LLM (Ollama)"
    case customShell = "Custom Shell"

    public var id: String { rawValue }

    public var defaultSessionName: String {
        switch self {
        case .claudeCode: return "claude"
        case .antigravity: return "antigravity"
        case .localLlm: return "ollama"
        case .customShell: return "shell"
        }
    }

    public var iconName: String {
        switch self {
        case .claudeCode: return "brain"
        case .antigravity: return "sparkles"
        case .localLlm: return "cpu"
        case .customShell: return "terminal"
        }
    }

    public var defaultCommand: String? {
        switch self {
        case .claudeCode: return "claude"
        case .antigravity: return "agy"
        case .localLlm: return "ollama run llama3"
        case .customShell: return nil
        }
    }
}

public struct TabSession: Identifiable, Equatable, Sendable, Codable {
    public let id: UUID
    public var title: String
    public var agentType: AgentType
    public var sessionName: String
    public var command: String?
    public var serverId: UUID?

    public init(
        id: UUID = UUID(),
        title: String,
        agentType: AgentType = .customShell,
        sessionName: String? = nil,
        command: String? = nil,
        serverId: UUID? = nil
    ) {
        self.id = id
        self.title = title
        self.agentType = agentType
        self.sessionName = sessionName ?? "term-\(id.uuidString.prefix(6).lowercased())"
        self.command = command
        self.serverId = serverId
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
