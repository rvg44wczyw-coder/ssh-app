import CryptoKit
import Foundation

/// Quick commands (the buttons above the keyboard) in the iCloud sync. A button has no stable id
/// across devices (every install seeds its own stock buttons), so the synced id is a digest of
/// the button's content: equal buttons collapse into one, an edited button is a new id and the
/// old one is deleted. Order is not synced.
struct SyncedQuickCommand: Codable, Equatable {
    var label: String
    var text: String
    var run: Bool
    var confirm: Bool
    var menuSource: String?
    var menuTemplate: String?
    var menuItems: String?

    static let prefix = "q_"

    init(_ c: QuickCommand) {
        label = c.label
        text = c.text
        run = c.run
        confirm = c.confirm
        menuSource = c.menu?.source.rawValue
        menuTemplate = c.menu?.template
        menuItems = c.menu?.items
    }

    var id: String {
        let joined = [label, text, run ? "1" : "0", confirm ? "1" : "0", menuSource ?? "", menuTemplate ?? "", menuItems ?? ""]
            .joined(separator: "\u{1f}")
        return SHA256.hash(data: Data(joined.utf8)).prefix(12).map { String(format: "%02x", $0) }.joined()
    }

    var command: QuickCommand {
        var menu: QuickCommandMenu?
        if let s = menuSource.flatMap(QuickCommandMenu.Source.init(rawValue:)) {
            menu = QuickCommandMenu(source: s, template: menuTemplate ?? "", items: menuItems ?? "")
        }
        return QuickCommand(label: label, text: text, run: run, confirm: confirm, menu: menu)
    }

    /// Only the buttons the user has saved leave the device; a button with something that looks
    /// like a secret stays.
    static func outgoing(_ commands: [QuickCommand]) -> [CollectionItem] {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        var seen = Set<String>()
        return commands.compactMap { c in
            let s = SyncedQuickCommand(c)
            guard !SyncedSnippet.looksSecret(c.text), !SyncedSnippet.looksSecret(c.label),
                  seen.insert(s.id).inserted,
                  let data = try? e.encode(s), let json = String(data: data, encoding: .utf8) else { return nil }
            return CollectionItem(id: s.id, payload: json)
        }
    }

    /// The other side is not trusted: a button types its text into a terminal, so the text is
    /// checked as a snippet is, and a menu may only offer typable choices.
    static func decode(_ item: CollectionItem) -> SyncedQuickCommand? {
        func plain(_ s: String, max: Int, allowEmpty: Bool = false) -> Bool {
            (allowEmpty || !s.isEmpty) && s.count <= max
                && !s.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
        }
        guard let s = try? JSONDecoder().decode(SyncedQuickCommand.self, from: Data(item.payload.utf8)),
              s.id == item.id, plain(s.label, max: 40), plain(s.text, max: 1000),
              !SyncedSnippet.looksSecret(s.text), !SyncedSnippet.looksSecret(s.label),
              s.menuSource == nil || QuickCommandMenu.Source(rawValue: s.menuSource ?? "") != nil,
              plain(s.menuTemplate ?? "", max: 1000, allowEmpty: true),
              !SyncedSnippet.looksSecret(s.menuTemplate ?? ""),
              (s.menuItems ?? "").count <= 4000,
              !(s.menuItems ?? "").unicodeScalars.contains(where: { $0 != "\n" && CharacterSet.controlCharacters.contains($0) })
        else { return nil }
        return s
    }
}

/// The accent colour in the iCloud sync: one item, written only when the user has chosen a colour.
enum SyncedAppearance {
    static let prefix = "a_"
    static let itemID = "accent"

    private struct Payload: Codable { var accent: String }

    static func outgoing(defaults: UserDefaults = .standard) -> [CollectionItem] {
        let raw = defaults.string(forKey: AppAccent.storageKey) ?? ""
        guard isValid(raw), let data = try? JSONEncoder().encode(Payload(accent: raw)),
              let json = String(data: data, encoding: .utf8) else { return [] }
        return [CollectionItem(id: itemID, payload: json)]
    }

    static func decode(_ item: CollectionItem) -> String? {
        guard item.id == itemID, let p = try? JSONDecoder().decode(Payload.self, from: Data(item.payload.utf8)),
              isValid(p.accent) else { return nil }
        return p.accent
    }

    /// A preset name or `#rrggbb`.
    static func isValid(_ raw: String) -> Bool {
        if AppAccent(rawValue: raw) != nil { return true }
        return raw.count == 7 && raw.hasPrefix("#") && raw.dropFirst().allSatisfy { $0.isHexDigit && $0.isASCII }
    }
}
