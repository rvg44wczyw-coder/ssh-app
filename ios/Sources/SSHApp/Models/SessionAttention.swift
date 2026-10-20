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
