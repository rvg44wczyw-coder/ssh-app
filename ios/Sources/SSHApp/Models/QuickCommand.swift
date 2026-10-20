#if canImport(UIKit)
import UIKit
#endif
import Foundation

/// A button on the bar above the keyboard that types a CLI command (`clear`, `/model`, ...).
/// The keys the system keyboard bar already has (Esc, Ctrl, Tab, arrows) are not repeated.
/// Long-press menu of a quick command: the command is typed with the chosen value in place of
/// `{choice}` (for example `/model {choice}`).
public struct QuickCommandMenu: Codable, Equatable {
    public enum Source: String, Codable, CaseIterable {
        /// Models of the agent running in the active tab (asked from the host).
        case agentModels
        /// The list written in `items`: one per line, `value` or `label = value`.
        case fixed
    }

    public var source: Source
    public var template: String
    public var items: String

    public init(source: Source, template: String, items: String = "") {
        self.source = source
        self.template = template
        self.items = items
    }

    /// `(label, value)` pairs of a fixed list; values that could not be typed safely are dropped.
    public var fixedChoices: [(label: String, value: String)] {
        items.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard let value = parts.last, QuickCommand.isTypableChoice(value) else { return nil }
            return (parts.count == 2 ? parts[0] : value, value)
        }
    }
}

public struct QuickCommand: Identifiable, Codable, Equatable {
    public var id = UUID()
    public var label: String
    public var text: String
    /// Press Return after the text.
    public var run: Bool
    /// Ask before sending; on by default for `clear` and `/clear`, which wipe the screen or the agent's context.
    public var confirm: Bool
    /// Choices shown on a long press (nil = none).
    public var menu: QuickCommandMenu?

    public init(label: String, text: String, run: Bool = true, confirm: Bool? = nil, menu: QuickCommandMenu? = nil) {
        self.label = label
        self.text = text
        self.run = run
        self.confirm = confirm ?? Self.isDestructive(text)
        self.menu = menu
    }

    /// A choice that may be typed into a terminal: no whitespace or control characters, no shell
    /// metacharacters, not starting with `-`.
    static func isTypableChoice(_ s: String) -> Bool {
        guard let first = s.unicodeScalars.first, s.count <= 100,
            CharacterSet.alphanumerics.contains(first), first.isASCII
        else { return false }
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._:/@-")
        return s.unicodeScalars.allSatisfy { allowed.contains($0) }
    }

    /// The bytes for choosing `value` from the menu.
    public func bytes(choosing value: String) -> [UInt8]? {
        guard let menu, Self.isTypableChoice(value) else { return nil }
        let template = menu.template.isEmpty ? text + " {choice}" : menu.template
        return Array((template.replacingOccurrences(of: "{choice}", with: value) + (run ? "\r" : "")).utf8)
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        label = try c.decode(String.self, forKey: .label)
        text = try c.decode(String.self, forKey: .text)
        run = try c.decodeIfPresent(Bool.self, forKey: .run) ?? true
        confirm = try c.decodeIfPresent(Bool.self, forKey: .confirm) ?? Self.isDestructive(text)
        menu = try c.decodeIfPresent(QuickCommandMenu.self, forKey: .menu)
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
        QuickCommand(label: "/model", text: "/model", menu: modelMenu),
        QuickCommand(label: "/help", text: "/help"),
        QuickCommand(label: "/compact", text: "/compact"),
        QuickCommand(label: "/status", text: "/status"),
    ]

    /// Long press on `/model` lists the models of the agent in the tab.
    static let modelMenu = QuickCommandMenu(source: .agentModels, template: "/model {choice}")

    /// The former stock button (its label is stored data, never drawn now), kept to recognise (and remove) it; the placeholder itself still
    /// works in custom commands.
    static let pasteButton = QuickCommand(label: "📋", text: QuickCommand.clipboardToken, run: false, confirm: false)

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key),
           let saved = try? JSONDecoder().decode([QuickCommand].self, from: data) {
            commands = saved
            // One-time: the stock /model button gets its model menu.
            if !UserDefaults.standard.bool(forKey: "quick_commands_model_menu_added") {
                for i in commands.indices where commands[i].text == "/model" && commands[i].menu == nil {
                    commands[i].menu = Self.modelMenu
                }
                UserDefaults.standard.set(true, forKey: "quick_commands_model_menu_added")
            }
            // One-time: the stock paste button is gone (owner, 2026-10-19). Only the exact stock
            // button is removed; a custom command that uses {{clipboard}} stays.
            if !UserDefaults.standard.bool(forKey: "quick_commands_paste_removed") {
                commands.removeAll { $0.label == Self.pasteButton.label && $0.text == QuickCommand.clipboardToken }
                UserDefaults.standard.set(true, forKey: "quick_commands_paste_removed")
            }
        } else {
            UserDefaults.standard.set(true, forKey: "quick_commands_paste_removed")
            UserDefaults.standard.set(true, forKey: "quick_commands_model_menu_added")
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

/// Model lists asked from the host, remembered for a few hours per agent so a long press opens at once.
@MainActor
final class ModelListCache {
    static let shared = ModelListCache()
    private var entries: [String: (at: Date, models: [(id: String, label: String)])] = [:]

    func cached(_ agent: String) -> [(id: String, label: String)]? {
        guard let e = entries[agent], Date().timeIntervalSince(e.at) < 6 * 3600 else { return nil }
        return e.models
    }

    func store(_ agent: String, _ models: [(id: String, label: String)]) {
        entries[agent] = (Date(), models)
    }
}
