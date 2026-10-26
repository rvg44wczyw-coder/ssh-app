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

/// The hosts a sandbox's proxy always lets through for an agent (Settings → Agents → Sandbox).
/// Kept in `UserDefaults`, one list per agent, and synced as one item per (agent, host).
enum SandboxAllowStore {
    static let agents = ["claude", "codex", "grok", "agy"]
    static let maxPerAgent = 48

    static func key(_ agent: String) -> String { "sandbox.always.\(agent)" }

    static func hosts(_ agent: String, defaults: UserDefaults = .standard) -> [String] {
        agents.contains(agent) ? (defaults.stringArray(forKey: key(agent)) ?? []) : []
    }

    static func set(_ hosts: [String], for agent: String, defaults: UserDefaults = .standard) {
        guard agents.contains(agent) else { return }
        defaults.set(Array(hosts.prefix(maxPerAgent)), forKey: key(agent))
    }

    /// A DNS name, or `*.` and a DNS name with at least two labels.
    static func normalized(_ raw: String) -> String? {
        let h = raw.trimmingCharacters(in: .whitespaces).lowercased()
        let body = h.hasPrefix("*.") ? String(h.dropFirst(2)) : h
        let labels = body.split(separator: ".", omittingEmptySubsequences: false)
        let ok = labels.count >= 2 && body.count <= 253 && labels.allSatisfy { l in
            !l.isEmpty && l.count <= 63 && !l.hasPrefix("-") && !l.hasSuffix("-")
                && l.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }
        }
        return ok ? h : nil
    }
}

/// The permanent sandbox hosts in the iCloud sync. The other side is not trusted: a host is only
/// accepted when it is a plain DNS name and the item id matches its content.
struct SyncedSandboxHost: Codable, Equatable {
    var agent: String
    var host: String

    static let prefix = "s_"

    var id: String { "\(Self.prefix)\(agent)_\(host)" }

    static func outgoing(defaults: UserDefaults = .standard) -> [CollectionItem] {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return SandboxAllowStore.agents.flatMap { agent in
            SandboxAllowStore.hosts(agent, defaults: defaults).compactMap { raw -> CollectionItem? in
                guard let host = SandboxAllowStore.normalized(raw) else { return nil }
                let s = SyncedSandboxHost(agent: agent, host: host)
                guard let data = try? e.encode(s), let json = String(data: data, encoding: .utf8) else { return nil }
                return CollectionItem(id: s.id, payload: json)
            }
        }
    }

    static func decode(_ item: CollectionItem) -> SyncedSandboxHost? {
        guard let s = try? JSONDecoder().decode(SyncedSandboxHost.self, from: Data(item.payload.utf8)),
              SandboxAllowStore.agents.contains(s.agent), SandboxAllowStore.normalized(s.host) == s.host,
              s.id == item.id else { return nil }
        return s
    }

    /// Adds the incoming hosts and drops the deleted ones, per agent.
    static func apply(upserts: [CollectionItem], deletes: [String], defaults: UserDefaults = .standard) {
        for agent in SandboxAllowStore.agents {
            var list = SandboxAllowStore.hosts(agent, defaults: defaults)
            let before = list
            list.removeAll { deletes.contains(SyncedSandboxHost(agent: agent, host: $0).id) }
            for item in upserts {
                guard let s = decode(item), s.agent == agent, !list.contains(s.host) else { continue }
                list.append(s.host)
            }
            if list != before { SandboxAllowStore.set(list, for: agent, defaults: defaults) }
        }
    }
}
