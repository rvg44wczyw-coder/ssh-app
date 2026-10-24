import Foundation

/// How an agent's terminal UI answers to the keys the app drives it with: the queue of messages
/// typed while it works, the suggestion for the next prompt, how a picture is named in a message.
/// Everything agent-specific that the feed does on its own lives here, one entry per agent: a new
/// agent (Grok, Codex, ...) is a new `static let` and a line in `resolve`, plus its screen dialect
/// in the Rust parser (`crates/core/src/screen`), not a new branch in the views.
///
/// Verified keys (live, private tmux, 2026-10-24): Claude Code takes Ctrl+X Ctrl+S for "send now";
/// Antigravity takes Alt+M ("alt+m to send immediately" in its queue). In both, Up brings the queued
/// messages back into the box and Ctrl+U empties it line by line.
struct AgentTraits: Equatable {
    let id: String
    /// Pushes a queued message now, interrupting the turn. `nil`: the agent has no such key.
    let sendNowKeys: [UInt8]?
    /// Brings the queued messages back into the input box.
    let pullQueueKeys: [UInt8]
    /// Empties the line the cursor is on.
    let clearLineKeys: [UInt8]
    /// Turns the grey suggestion of the next prompt into real text. `nil`: the agent shows none.
    let acceptSuggestionKeys: [UInt8]?
    /// Put before the path of a picture in a message ("@" makes Claude Code read it).
    let imageReferencePrefix: String

    static let claude = AgentTraits(id: "claude", sendNowKeys: [0x18, 0x13], pullQueueKeys: [0x1b, 0x5b, 0x41],
                                    clearLineKeys: [0x15], acceptSuggestionKeys: [0x09], imageReferencePrefix: "@")
    static let antigravity = AgentTraits(id: "agy", sendNowKeys: [0x1b, 0x6d], pullQueueKeys: [0x1b, 0x5b, 0x41],
                                         clearLineKeys: [0x15], acceptSuggestionKeys: nil, imageReferencePrefix: "")
    /// An agent we have not checked: nothing is assumed beyond the keys every terminal editor has.
    static let generic = AgentTraits(id: "generic", sendNowKeys: nil, pullQueueKeys: [0x1b, 0x5b, 0x41],
                                     clearLineKeys: [0x15], acceptSuggestionKeys: nil, imageReferencePrefix: "")

    /// The traits of the agent in a tab: a tab made for an agent, or one it was started in by hand.
    static func resolve(agentType: AgentType, detectedAgy: Bool, detectedClaude: Bool = false) -> AgentTraits {
        if detectedAgy { return .antigravity }
        if detectedClaude { return .claude }
        switch agentType {
        case .claudeCode: return .claude
        case .antigravity: return .antigravity
        case .localLlm, .customShell, .dockerContainer: return .generic
        }
    }
}
