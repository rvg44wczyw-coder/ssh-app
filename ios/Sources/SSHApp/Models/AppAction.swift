import SwiftUI

/// Every action that used to be a toolbar icon. The action panel, the long-press hint and the
/// accessibility labels all read from this one list.
public enum AppAction: String, CaseIterable, Identifiable, Sendable {
    case chatToggle
    case tmuxSessions
    case aiAssistant
    case snippets
    case agentProfiles
    case fileBrowser
    case telemetry
    case webPreview
    case diffViewer
    case publicKey
    case settings

    public var id: String { rawValue }

    /// Short label under the icon. The chat tile names the mode it will switch *to*.
    public func title(isChatMode: Bool) -> String {
        switch self {
        case .chatToggle: return isChatMode ? "Terminal" : "Chat"
        case .tmuxSessions: return "tmux sessions"
        case .aiAssistant: return "AI assistant"
        case .snippets: return "Snippets"
        case .agentProfiles: return "Agents"
        case .fileBrowser: return "Files"
        case .telemetry: return "Host monitor"
        case .webPreview: return "Web preview"
        case .diffViewer: return "Git diff"
        case .publicKey: return "Public key"
        case .settings: return "Settings"
        }
    }

    public func systemImage(isChatMode: Bool, keyVisible: Bool) -> String {
        switch self {
        case .chatToggle: return isChatMode ? "terminal.fill" : "bubble.left.and.bubble.right.fill"
        case .tmuxSessions: return "rectangle.stack"
        case .aiAssistant: return "wand.and.stars"
        case .snippets: return "bolt.fill"
        case .agentProfiles: return "play.rectangle.on.rectangle.fill"
        case .fileBrowser: return "folder.fill"
        case .telemetry: return "gauge.with.dots.needle.33percent"
        case .webPreview: return "globe"
        case .diffViewer: return "arrow.triangle.pull"
        case .publicKey: return keyVisible ? "key.fill" : "key"
        case .settings: return "gearshape"
        }
    }

    public var tint: Color {
        switch self {
        case .chatToggle: return .blue
        case .tmuxSessions: return .indigo
        case .aiAssistant: return .purple
        case .snippets: return .orange
        case .agentProfiles: return .mint
        case .fileBrowser: return .yellow
        case .telemetry: return .green
        case .webPreview: return .cyan
        case .diffViewer: return .pink
        case .publicKey: return .teal
        case .settings: return .gray
        }
    }

    /// Text shown on long-press (and read by VoiceOver as the accessibility hint).
    public var hint: String {
        switch self {
        case .chatToggle:
            return "Switch this tab between the raw terminal and a readable chat-style transcript of the agent session."
        case .tmuxSessions:
            return "List, open and close the tmux sessions that keep running on the host."
        case .aiAssistant:
            return "Describe what you want in words; the local Ollama model on the host turns it into a shell command you can review before running."
        case .snippets:
            return "Saved commands and fuzzy search over your shell history."
        case .agentProfiles:
            return "Start Claude Code, Antigravity and others in a new tab, choose what starts automatically, and manage their approval hooks."
        case .fileBrowser:
            return "Browse the host's folders, preview files, and insert a path into the terminal."
        case .telemetry:
            return "CPU, memory, disk and running agent processes on the host."
        case .webPreview:
            return "Open a dev server running on the host (localhost ports) inside the app."
        case .diffViewer:
            return "Review the uncommitted changes of a repository on the host."
        case .publicKey:
            return "Show or hide the key banner so you can copy this phone's public SSH key to a host."
        case .settings:
            return "Servers, SSH key and app options."
        }
    }

    /// Needs a live connection to the active host (shown disabled otherwise, never hidden).
    public var requiresConnection: Bool {
        switch self {
        case .chatToggle, .aiAssistant, .snippets, .agentProfiles, .fileBrowser, .telemetry, .webPreview:
            return true
        case .tmuxSessions, .diffViewer, .publicKey, .settings:
            return false
        }
    }
}
