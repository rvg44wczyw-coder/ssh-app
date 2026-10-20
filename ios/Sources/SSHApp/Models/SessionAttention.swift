import SwiftUI

/// What a session wants from the user, shown as a small symbol on its card and tab (instead of a
/// banner over the screen). Most urgent first.
public enum SessionAttention: Int, Comparable {
    case finished, needsYou, question, approval

    public static func < (a: SessionAttention, b: SessionAttention) -> Bool { a.rawValue < b.rawValue }

    public var symbol: String {
        switch self {
        case .approval: return "exclamationmark.shield.fill"
        case .question: return "questionmark.bubble.fill"
        case .needsYou: return "bell.fill"
        case .finished: return "checkmark.circle.fill"
        }
    }

    public var color: Color {
        switch self {
        case .approval: return .red
        case .question: return .orange
        case .needsYou: return .blue
        case .finished: return .green
        }
    }

    public var label: String {
        switch self {
        case .approval: return "Waiting for your approval"
        case .question: return "Asks a question"
        case .needsYou: return "Needs you"
        case .finished: return "Finished"
        }
    }
}

/// The symbol itself, one look everywhere.
/// A small symbol for what a request is about, from the agent's tool name (the card shows it
/// beside the approval badge).
public enum RequestKind {
    public static func symbol(tool: String) -> String {
        switch tool.lowercased() {
        case "bash", "run_command", "shell", "run_shell_command", "bash_output": return "terminal"
        case "edit", "write", "multiedit", "notebookedit", "write_to_file", "replace_file_content",
             "multi_replace_file_content", "edit_file", "write_file", "replace", "patch", "apply_patch":
            return "square.and.pencil"
        case "read", "view_file", "read_file", "read_many_files", "view": return "doc.text"
        case "webfetch", "websearch", "search_web", "read_url_content": return "globe"
        case "grep", "glob", "grep_search", "find_by_name", "list_dir", "ls": return "magnifyingglass"
        default: return "wrench.and.screwdriver"
        }
    }
}

public struct SessionAttentionBadge: View {
    let attention: SessionAttention
    var size: CGFloat = 12

    public init(attention: SessionAttention, size: CGFloat = 12) {
        self.attention = attention
        self.size = size
    }

    public var body: some View {
        Image(systemName: attention.symbol)
            .font(.system(size: size, weight: .bold))
            .foregroundColor(attention.color)
            .accessibilityLabel(attention.label)
    }
}
