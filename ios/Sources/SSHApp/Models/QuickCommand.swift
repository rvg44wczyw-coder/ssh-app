import Foundation

/// A button on the bar above the keyboard that types a CLI command (`clear`, `/model`, ...).
/// The keys the system keyboard bar already has (Esc, Ctrl, Tab, arrows) are not repeated.
public struct QuickCommand: Identifiable, Codable, Equatable {
    public var id = UUID()
    public var label: String
    public var text: String
    /// Press Return after the text.
    public var run: Bool
    /// Ask before sending; on by default for `clear` and `/clear`, which wipe the screen or the agent's context.
    public var confirm: Bool

    public init(label: String, text: String, run: Bool = true, confirm: Bool? = nil) {
        self.label = label
        self.text = text
        self.run = run
        self.confirm = confirm ?? Self.isDestructive(text)
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        label = try c.decode(String.self, forKey: .label)
        text = try c.decode(String.self, forKey: .text)
        run = try c.decodeIfPresent(Bool.self, forKey: .run) ?? true
        confirm = try c.decodeIfPresent(Bool.self, forKey: .confirm) ?? Self.isDestructive(text)
    }

    static func isDestructive(_ text: String) -> Bool {
        ["clear", "/clear"].contains(text.trimmingCharacters(in: .whitespaces))
    }

    public var bytes: [UInt8] { Array((text + (run ? "\r" : "")).utf8) }
}

@MainActor
public final class QuickCommandStore: ObservableObject {
    public static let shared = QuickCommandStore()
    private static let key = "keyboard_quick_commands"

    @Published public var commands: [QuickCommand] {
        didSet { save() }
    }

    public static let defaults: [QuickCommand] = [
        QuickCommand(label: "clear", text: "clear"),
        QuickCommand(label: "/clear", text: "/clear"),
        QuickCommand(label: "/model", text: "/model"),
        QuickCommand(label: "/help", text: "/help"),
        QuickCommand(label: "/compact", text: "/compact"),
        QuickCommand(label: "/status", text: "/status"),
    ]

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key),
           let saved = try? JSONDecoder().decode([QuickCommand].self, from: data) {
            commands = saved
        } else {
            commands = Self.defaults
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(commands) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }

    public func resetToDefaults() { commands = Self.defaults }
}
