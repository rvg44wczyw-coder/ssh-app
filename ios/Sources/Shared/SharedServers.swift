import Foundation
import Security

/// A server as the Files extension needs it. The app publishes the list into a Keychain item of the
/// shared access group (no App Group needed); the extension reads it together with the private key.
/// Only servers with a pinned host key are published: the extension never trusts a key on first use.
public struct SharedServer: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var host: String
    public var port: UInt16
    public var username: String
    public var keyId: String?
    public var hostKey: String
    /// Folder shown as the root of the server in Files (`nil` = the login user's home).
    public var startFolder: String?

    public init(id: String, name: String, host: String, port: UInt16, username: String,
                keyId: String?, hostKey: String, startFolder: String?) {
        self.id = id
        self.name = name
        self.host = host
        self.port = port
        self.username = username
        self.keyId = keyId
        self.hostKey = hostKey
        self.startFolder = startFolder
    }
}

public enum SharedStore {
    private static let service = "com.sshapp.shared"
    private static let account = "servers"
    private static let keyService = "com.sshapp.keys"

    private static func base(service: String, account: String, group: String?) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        if let group { q[kSecAttrAccessGroup as String] = group }
        return q
    }

    /// App side: replaces the published list.
    @discardableResult
    public static func publish(_ servers: [SharedServer]) -> Bool {
        guard let data = try? JSONEncoder().encode(servers) else { return false }
        SecItemDelete(base(service: service, account: account, group: nil) as CFDictionary)
        var q = base(service: service, account: account, group: nil)
        q[kSecValueData as String] = data
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        q[kSecAttrSynchronizable as String] = kCFBooleanFalse as Any
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess
    }

    public static func load(group: String?) -> [SharedServer] {
        var q = base(service: service, account: account, group: group)
        q[kSecReturnData as String] = kCFBooleanTrue as Any
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let list = try? JSONDecoder().decode([SharedServer].self, from: data) else { return [] }
        return list
    }

    /// The private key of `keyId` (`nil` = the default key), same accounts as `KeychainManager`.
    public static func privateKey(id: String?, group: String?) -> String? {
        let acct = id.map { "ed25519_key_\($0)" } ?? "ed25519_user_key"
        var q = base(service: keyService, account: acct, group: group)
        q[kSecReturnData as String] = kCFBooleanTrue as Any
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
