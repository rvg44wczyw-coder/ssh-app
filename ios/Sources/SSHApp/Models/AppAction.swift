import SwiftUI

/// Every action that used to be a toolbar icon. The action panel, the long-press hint and the
/// accessibility labels all read from this one list.
public enum AppAction: String, CaseIterable, Identifiable, Sendable {
    case chatToggle
    case tmuxSessions
    case aiAssistant
    case snippets
    case commands
    case sessionPermissions
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
        case .commands: return "Commands"
        case .sessionPermissions: return "Permissions"
        case .agentProfiles: return "Agents"
        case .fileBrowser: return "Files"
        case .telemetry: return "Host monitor"
        case .webPreview: return "Web preview"
        case .diffViewer: return "Git log & changes"
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
        case .commands: return "slash.circle"
        case .sessionPermissions: return "checkmark.shield"
        case .agentProfiles: return "play.rectangle.on.rectangle.fill"
        case .fileBrowser: return "folder.fill"
        case .telemetry: return "gauge.with.dots.needle.33percent"
        case .webPreview: return "globe"
        case .diffViewer: return "clock.arrow.circlepath"
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
        case .commands: return .brown
        case .sessionPermissions: return .red
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
            return "List, open and close all tmux sessions that keep running on the host (the whole server, not just this tab)."
        case .aiAssistant:
            return "Describe what you want in words; the local Ollama model on the host turns it into a shell command you can review before running."
        case .snippets:
            return "Saved commands and fuzzy search over your shell history."
        case .commands:
            return "Slash commands of the agent in this tab (/model, /config, /compact, ...): insert at the prompt or run in one tap."
        case .sessionPermissions:
            return "What this tab's agent may do: its folder, the rules in effect (profile and project), the start mode, and suggestions such as accepting edits in the project folder."
        case .agentProfiles:
            return "Start Claude Code, Antigravity and others in a new tab, choose what starts automatically, and manage their approval hooks."
        case .fileBrowser:
            return "Browse the host's folders, preview files, and insert a path into the terminal."
        case .telemetry:
            return "CPU, memory, disk and running agent processes on the host."
        case .webPreview:
            return "Open a dev server running on the host (localhost ports) inside the app."
        case .diffViewer:
            return "Browse the commit history of the tab's repository and see its uncommitted changes."
        case .publicKey:
            return "Show and copy this phone's public SSH keys to add them to a host's authorized_keys."
        case .settings:
            return "Servers, SSH key and app options."
        }
    }

    /// Belongs to the current session (its own toolbar button) rather than the quick menu.
    public var isSessionAction: Bool {
        switch self {
        case .chatToggle, .sessionPermissions, .agentProfiles, .diffViewer: return true
        default: return false
        }
    }

    /// Needs a live connection to the active host (shown disabled otherwise, never hidden).
    public var requiresConnection: Bool {
        switch self {
        case .chatToggle, .aiAssistant, .snippets, .commands, .sessionPermissions, .agentProfiles, .fileBrowser, .telemetry, .webPreview, .diffViewer:
            return true
        case .tmuxSessions, .publicKey, .settings:
            return false
        }
    }
}

/// Where the AI assistant shows up (quick-menu tile and the key above the keyboard).
/// Stored as `ai_assistant_visibility`; before that setting existed only the boolean
/// `always_show_ai_assistant` was stored, which is honoured until the new one is chosen.
public enum AiAssistantVisibility: String, CaseIterable, Identifiable {
    /// Only while connected to a host (it needs one).
    case auto
    /// Also without a connection (tile disabled until connected).
    case always
    /// Nowhere.
    case never

    public var id: String { rawValue }
    public static let storageKey = "ai_assistant_visibility"

    public static func resolved(raw: String, legacyAlways: Bool) -> AiAssistantVisibility {
        AiAssistantVisibility(rawValue: raw) ?? (legacyAlways ? .always : .auto)
    }

    public func isVisible(connected: Bool) -> Bool {
        switch self {
        case .auto: return connected
        case .always: return true
        case .never: return false
        }
    }
}

extension AppAction {
    /// Settings is the way back to everything else, so it can never be hidden.
    public var canHide: Bool { self != .settings }
}

/// Which quick-menu tiles the user hid, persisted as a comma-separated list of action ids
/// (`@AppStorage("hidden_quick_actions")`). Unknown ids (from another app version) are ignored
/// and kept out of the result, and non-hideable actions can never end up in the set.
public enum QuickMenuPrefs {
    public static let storageKey = "hidden_quick_actions"

    public static func hidden(from raw: String) -> Set<AppAction> {
        Set(raw.split(separator: ",").compactMap { AppAction(rawValue: String($0)) }.filter(\.canHide))
    }

    public static func raw(from hidden: Set<AppAction>) -> String {
        AppAction.allCases.filter { hidden.contains($0) && $0.canHide }.map(\.rawValue).joined(separator: ",")
    }

    public static func toggled(_ action: AppAction, in raw: String) -> String {
        guard action.canHide else { return raw }
        var set = hidden(from: raw)
        if set.contains(action) { set.remove(action) } else { set.insert(action) }
        return Self.raw(from: set)
    }
}

extension QuickMenuPrefs {
    public static let orderKey = "quick_action_order"

    /// The saved order first (unknown and duplicate ids dropped), then whatever is not in it, in the
    /// built-in order: a new action of a later app version shows up at the end.
    public static func ordered(from raw: String) -> [AppAction] {
        var seen = Set<AppAction>()
        var out: [AppAction] = []
        for part in raw.split(separator: ",") {
            if let a = AppAction(rawValue: String(part)), !seen.contains(a) { seen.insert(a); out.append(a) }
        }
        out += AppAction.allCases.filter { !seen.contains($0) }
        return out
    }

    public static func raw(from order: [AppAction]) -> String { order.map(\.rawValue).joined(separator: ",") }

    /// `action` goes to the place of `target` (the others shift).
    public static func moved(_ action: AppAction, to target: AppAction, in raw: String) -> String {
        var order = ordered(from: raw)
        guard action != target, let from = order.firstIndex(of: action), let to = order.firstIndex(of: target) else { return raw }
        order.remove(at: from)
        order.insert(action, at: to)
        return Self.raw(from: order)
    }
}

/// Size of a menu's grid, set in Settings → Appearance: how many tiles to a row and how big the
/// icon of a tile is. One setting each for the quick menu and for the session menu.
public enum MenuGridPrefs {
    public static let columnRange = 2...6
    public static let sizeRange: ClosedRange<Double> = 28...60
    public static let defaultColumns = 4
    public static let defaultSize = 40.0

    /// Columns as shown: kept in range, two for accessibility text sizes.
    public static func shownColumns(_ chosen: Int, accessibility: Bool) -> Int {
        accessibility ? 2 : min(max(chosen, columnRange.lowerBound), columnRange.upperBound)
    }

    /// A tile's cell: the icon plus room for the hide badge in edit mode (tiles have no label).
    public static func cellWidth(iconSize: Double) -> CGFloat { CGFloat(iconSize + 14) }

    /// Width of a menu panel: columns of cells, the gaps, the padding.
    public static func panelWidth(columns: Int, iconSize: Double) -> CGFloat {
        max(CGFloat(columns) * cellWidth(iconSize: iconSize) + CGFloat(columns - 1) * 6 + 28, 190)
    }

    public static func columnsKey(session: Bool) -> String { session ? "session_menu_columns" : "quick_menu_columns" }
    public static func sizeKey(session: Bool) -> String { session ? "session_menu_icon_size" : "quick_menu_icon_size" }
}
