import Foundation

/// Free-text labels the owner puts on project folders ("work", "personal"…), per server (spec 61
/// §5.0.1). They only group the Hooks screen and its bulk actions; they never change where hooks
/// are written. Kept on the phone.
enum HookCategories {
    static let key = "hooks.categories"
    static let other = "Other"
    static let maxLength = 24
    static let maxEntries = 256

    static func normalized(_ text: String) -> String? {
        let flat = text.unicodeScalars.map { $0.properties.generalCategory == .control ? " " : String($0) }.joined()
        let t = String(flat.trimmingCharacters(in: .whitespaces).prefix(maxLength))
        return t.isEmpty || t.caseInsensitiveCompare(other) == .orderedSame ? nil : t
    }

    private static func id(_ server: UUID?, _ folder: String) -> String {
        "\(server?.uuidString ?? "default")|\(folder)"
    }

    static func all(_ defaults: UserDefaults = .standard) -> [String: String] {
        defaults.dictionary(forKey: key) as? [String: String] ?? [:]
    }

    static func category(server: UUID?, folder: String, in map: [String: String]) -> String {
        map[id(server, folder)] ?? other
    }

    /// An empty or invalid label removes the category.
    static func set(_ text: String, server: UUID?, folder: String, _ defaults: UserDefaults = .standard) {
        var map = all(defaults)
        if let c = normalized(text) {
            guard map[id(server, folder)] != nil || map.count < maxEntries else { return }
            map[id(server, folder)] = c
        } else {
            map.removeValue(forKey: id(server, folder))
        }
        defaults.set(map, forKey: key)
    }

    /// Categories in use for the given folders, sorted, "Other" last; folders keep their order.
    static func grouped(_ folders: [String], server: UUID?, in map: [String: String]) -> [(category: String, folders: [String])] {
        var by: [String: [String]] = [:]
        for f in folders { by[category(server: server, folder: f, in: map), default: []].append(f) }
        let names = by.keys.sorted { a, b in
            if a == other { return false }
            if b == other { return true }
            return a.localizedCaseInsensitiveCompare(b) == .orderedAscending
        }
        return names.map { ($0, by[$0] ?? []) }
    }
}
