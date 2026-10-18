#if canImport(UIKit)
import UIKit
#endif
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

    /// Placeholder in `text` that is replaced by the clipboard text when the button is tapped.
    public static let clipboardToken = "{{clipboard}}"

    public var bytes: [UInt8] { Array((text + (run ? "\r" : "")).utf8) }

    /// The bytes to send now: `{{clipboard}}` is replaced by the pasteboard text (line breaks
    /// become Enter, as in a terminal paste). Nil when the command needs the clipboard and it is empty.
    @MainActor
    public func resolvedBytes() -> [UInt8]? {
        guard text.contains(Self.clipboardToken) else { return bytes }
        #if canImport(UIKit)
        guard UIPasteboard.general.hasStrings, let clip = UIPasteboard.general.string, !clip.isEmpty
        else { return nil }
        let flat = clip.replacingOccurrences(of: "\r\n", with: "\r").replacingOccurrences(of: "\n", with: "\r")
        return Array((text.replacingOccurrences(of: Self.clipboardToken, with: flat) + (run ? "\r" : "")).utf8)
        #else
        return nil
        #endif
    }
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
        QuickCommandStore.pasteButton,
    ]

    /// Sends the clipboard text without Return.
    static let pasteButton = QuickCommand(label: "📋", text: QuickCommand.clipboardToken, run: false, confirm: false)

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key),
           let saved = try? JSONDecoder().decode([QuickCommand].self, from: data) {
            commands = saved
            // One-time: people who already customised the bar get the paste button too.
            if !UserDefaults.standard.bool(forKey: "quick_commands_paste_added") {
                if !saved.contains(where: { $0.text == QuickCommand.clipboardToken }) {
                    commands.append(Self.pasteButton)
                }
                UserDefaults.standard.set(true, forKey: "quick_commands_paste_added")
            }
        } else {
            UserDefaults.standard.set(true, forKey: "quick_commands_paste_added")
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
