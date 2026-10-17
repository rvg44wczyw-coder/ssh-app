import Foundation

/// Folder on the host in which new plain terminals start (agent tabs use their profile's folder).
/// Created when missing; sessions that already exist keep where they are.
public enum TerminalFolder {
    public static let storageKey = "default_terminal_dir"
    public static let fallback = "~/Terminals"

    /// The stored value, or the default when nothing was ever chosen. Empty = the host's home.
    public static var stored: String {
        UserDefaults.standard.object(forKey: storageKey) as? String ?? fallback
    }

    /// A value the core accepts (`~`, `~/x` or an absolute path, no control characters), or nil
    /// for "home / not usable". Never throws: a bad value just means the default location.
    public static func normalized(_ raw: String) -> String? {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, t.count <= 400, !t.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return nil }
        guard t == "~" || t.hasPrefix("~/") || t.hasPrefix("/") else { return nil }
        return t
    }

    public static func problem(_ raw: String) -> String? {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty || normalized(t) != nil ? nil : "Use a path that starts with ~/ or /"
    }
}
