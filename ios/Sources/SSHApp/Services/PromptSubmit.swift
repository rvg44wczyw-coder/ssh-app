import Foundation

/// How a message goes into the agent's input box: the one place that says what "sent" means.
public enum SubmitMode: Equatable {
    /// Type it and press Enter; a working agent queues it.
    case normal
    /// Stop the running turn first (Esc), then type and send at once.
    case interrupt
}

public enum SubmitOutcome: Equatable {
    /// The box took it; `queued` when the agent was working and holds it in its queue.
    case sent(queued: Bool)
    /// Nothing went out; the box was emptied so the text cannot be glued to the next message.
    case notSent
}

/// Reading the box back after Enter. The box is the truth: what the app believes it typed is not.
enum BoxCheck {
    /// The first characters of what was sent, whitespace collapsed (the screen may wrap lines).
    static func head(of sent: String) -> String {
        String(squashed(sent).prefix(24))
    }

    static func squashed(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// `true` when the box still holds the message that was just sent: it starts with the sent text.
    /// A hint, a suggestion or what the user typed at the laptop does not count, and an unreadable
    /// screen (`nil`) is taken as "gone" so nothing is pressed blindly.
    static func stillHolds(box: String?, sent: String) -> Bool {
        let head = head(of: sent)
        guard !head.isEmpty, let box else { return false }
        return squashed(box).hasPrefix(head)
    }
}
