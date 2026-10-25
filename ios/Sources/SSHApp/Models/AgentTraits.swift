import Foundation
import SshCoreBridge

/// How an agent's terminal UI answers to the keys the app drives it with: the queue of messages
/// typed while it works, the suggestion for the next prompt, how a picture is named in a message.
/// The table itself lives in the Rust core (`crates/core/src/agent_registry.rs`, one row per agent); this
/// struct is how the feed reads a row. A new agent (Grok, Codex, ...) is a new row there plus its two
/// adapters, not a new branch in the views. `AgentResolver` maps a tab to its row.
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

    init(id: String, sendNowKeys: [UInt8]?, pullQueueKeys: [UInt8], clearLineKeys: [UInt8],
         acceptSuggestionKeys: [UInt8]?, imageReferencePrefix: String) {
        self.id = id
        self.sendNowKeys = sendNowKeys
        self.pullQueueKeys = pullQueueKeys
        self.clearLineKeys = clearLineKeys
        self.acceptSuggestionKeys = acceptSuggestionKeys
        self.imageReferencePrefix = imageReferencePrefix
    }

    /// The keys of a row of the core's registry.
    init(descriptor d: AgentDescriptor) {
        self.init(id: d.id,
                  sendNowKeys: d.keys.sendNowKeys.map { [UInt8]($0) },
                  pullQueueKeys: [UInt8](d.keys.pullQueueKeys),
                  clearLineKeys: [UInt8](d.keys.clearLineKeys),
                  acceptSuggestionKeys: d.keys.acceptSuggestionKeys.map { [UInt8]($0) },
                  imageReferencePrefix: d.keys.imageReferencePrefix)
    }

    static let claude = AgentTraits(descriptor: AgentResolver.entry("claude"))
    static let antigravity = AgentTraits(descriptor: AgentResolver.entry("agy"))
    /// An agent we have not checked: nothing is assumed beyond the keys every terminal editor has.
    static let generic = AgentTraits(descriptor: AgentResolver.entry("generic"))
}

/// Which row of the core's registry a tab is: what runs in it (read from the screen) wins over what the
/// tab was made for. The one place that maps the UI's `AgentType` to an agent; the rest of the feed asks
/// the descriptor.
enum AgentResolver {
    /// The whole registry, read once: the rows never change while the app runs.
    static let all: [AgentDescriptor] = agentDescriptors()

    /// A registry row by id; the generic row when the id is unknown (never missing: the core has it).
    static func entry(_ id: String) -> AgentDescriptor {
        all.first { $0.id == id } ?? genericEntry
    }

    static let genericEntry: AgentDescriptor = genericAgentDescriptor()

    /// The row of the agent the screen shows, `nil` when no known agent is recognised.
    static func detected(dialect: ScreenDialect) -> AgentDescriptor? {
        guard dialect != .unknown else { return nil }
        return all.first { $0.dialect == dialect }
    }

    static func descriptor(agentType: AgentType, dialect: ScreenDialect) -> AgentDescriptor {
        if let found = detected(dialect: dialect) { return found }
        switch agentType {
        case .claudeCode: return entry("claude")
        case .antigravity: return entry("agy")
        case .localLlm, .customShell, .dockerContainer: return genericEntry
        }
    }
}

/// Which agent's screen a tab shows, steadied against text that only looks like another agent's screen
/// (a `cat` of a hostile file, prompt injection): going from one recognised agent straight to a different one
/// needs two looks in a row; from or to "nothing recognised" it is immediate.
struct DialectTracker {
    private(set) var current: ScreenDialect = .unknown
    private var candidate: ScreenDialect?

    mutating func observe(_ seen: ScreenDialect) -> ScreenDialect {
        if seen == current || seen == .unknown || current == .unknown {
            current = seen
            candidate = nil
        } else if candidate == seen {
            current = seen
            candidate = nil
        } else {
            candidate = seen
        }
        return current
    }
}
