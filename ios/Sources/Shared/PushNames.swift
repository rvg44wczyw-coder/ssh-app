import Foundation
import Security

/// What the phone knows about the thing a push's opaque `sid` stands for. The relay and Apple only ever see the `sid`; the
/// app publishes `sid → name` into a Keychain item of the shared access group (no App Group
/// needed, encrypted at rest) and the notification extension swaps the name into the banner on the
/// phone. Names never leave the device.
public enum PushNames {
    /// A tab or sandbox. `server`, `engine` and `sandbox` are set for a sandbox, so the extension
    /// can ask its server which hosts the proxy refused.
    public struct Entry: Codable, Equatable, Sendable {
        public var name: String
        public var server: String?
        public var engine: String?
        public var sandbox: String?

        public init(name: String, server: String? = nil, engine: String? = nil, sandbox: String? = nil) {
            self.name = name
            self.server = server
            self.engine = engine
            self.sandbox = sandbox
        }
    }

    private static let service = "com.sshapp.pushnames"
    private static let account = "names"
    /// A longer name is cut so a banner stays readable.
    public static let maxLength = 40
    /// The most names kept: closed tabs fall off, the list stays small.
    public static let maxEntries = 64

    private static func base(group: String?) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        if let group { q[kSecAttrAccessGroup as String] = group }
        return q
    }

    /// The text for a banner: control characters out, cut to `maxLength`.
    public static func clean(_ name: String) -> String {
        let flat = name.unicodeScalars.map { $0.properties.generalCategory == .control ? " " : String($0) }.joined()
        return String(flat.trimmingCharacters(in: .whitespaces).prefix(maxLength))
    }

    /// App side: replaces the published list.
    @discardableResult
    public static func publish(_ names: [String: Entry]) -> Bool {
        let kept = Dictionary(uniqueKeysWithValues: names.prefix(maxEntries).map { key, entry in
            var e = entry
            e.name = clean(entry.name)
            return (key, e)
        })
        guard let data = try? JSONEncoder().encode(kept) else { return false }
        SecItemDelete(base(group: nil) as CFDictionary)
        var q = base(group: nil)
        q[kSecValueData as String] = data
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        q[kSecAttrSynchronizable as String] = kCFBooleanFalse as Any
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess
    }

    public static func load(group: String?) -> [String: Entry] {
        var q = base(group: group)
        q[kSecReturnData as String] = kCFBooleanTrue as Any
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let map = try? JSONDecoder().decode([String: Entry].self, from: data) else { return [:] }
        return map
    }

    /// The banner body: "name: text" when the `sid` is known, else the relay's own text.
    public static func body(sid: String?, text: String, names: [String: Entry]) -> String {
        guard let sid, let name = names[sid]?.name, !name.isEmpty else { return text }
        return "\(name): \(text)"
    }

    /// The body of a blocked-host push once the server named the hosts: "name: blocked a.com and 2
    /// more". With no hosts the plain body stays.
    public static func blockedBody(name: String, hosts: [String], text: String) -> String {
        guard let first = hosts.first else { return "\(name): \(text)" }
        let more = hosts.count > 1 ? " and \(hosts.count - 1) more" : ""
        return "\(name): blocked \(clean(first))\(more)"
    }
}
