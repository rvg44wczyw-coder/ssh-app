import Foundation
import LocalAuthentication
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
    /// The key is read from the guarded copy, which asks for Face ID (or the passcode) each time.
    public var guarded: Bool

    public init(id: String, name: String, host: String, port: UInt16, username: String,
                keyId: String?, hostKey: String, startFolder: String?, guarded: Bool = false) {
        self.id = id
        self.name = name
        self.host = host
        self.port = port
        self.username = username
        self.keyId = keyId
        self.hostKey = hostKey
        self.startFolder = startFolder
        self.guarded = guarded
    }

    // A list published by an older app has no `guarded`: that means not guarded.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        host = try c.decode(String.self, forKey: .host)
        port = try c.decode(UInt16.self, forKey: .port)
        username = try c.decode(String.self, forKey: .username)
        keyId = try c.decodeIfPresent(String.self, forKey: .keyId)
        hostKey = try c.decode(String.self, forKey: .hostKey)
        startFolder = try c.decodeIfPresent(String.self, forKey: .startFolder)
        guarded = try c.decodeIfPresent(Bool.self, forKey: .guarded) ?? false
    }
}

public enum SharedStore {
    private static let service = "com.sshapp.shared"
    private static let account = "servers"
    private static let keyService = "com.sshapp.keys"
    /// Copies of the keys that need the owner's presence to be read (Face ID or the passcode).
    private static let guardedService = "com.sshapp.filekeys"

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

    private static func account(_ id: String?) -> String {
        id.map { "ed25519_key_\($0)" } ?? "ed25519_user_key"
    }

    /// App side: rebuilds the guarded copies. Always starts from none, so a key that is no longer
    /// used by a server in Files, or a switch that was turned off, leaves nothing behind.
    @discardableResult
    public static func publishGuardedKeys(ids: [String?], enabled: Bool) -> Bool {
        var wipe: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: guardedService]
        wipe[kSecUseAuthenticationUI as String] = kSecUseAuthenticationUISkip
        SecItemDelete(wipe as CFDictionary)
        guard enabled else { return true }
        guard let acl = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .userPresence, nil) else { return false }
        var ok = true
        var done = Set<String>()
        for id in ids where done.insert(account(id)).inserted {
            guard let pem = privateKey(id: id, group: nil), let data = pem.data(using: .utf8) else { ok = false; continue }
            var q = base(service: guardedService, account: account(id), group: nil)
            q[kSecValueData as String] = data
            q[kSecAttrAccessControl as String] = acl
            q[kSecAttrSynchronizable as String] = kCFBooleanFalse as Any
            if SecItemAdd(q as CFDictionary, nil) != errSecSuccess { ok = false }
        }
        return ok
    }

    /// The private key of `keyId` (`nil` = the default key), same accounts as `KeychainManager`.
    /// `guarded` reads the guarded copy through `context` (the system asks the owner) and never
    /// falls back to the plain key.
    public static func privateKey(id: String?, group: String?, guarded: Bool = false, context: LAContext? = nil) -> String? {
        let acct = account(id)
        var q = base(service: guarded ? guardedService : keyService, account: acct, group: group)
        if guarded, let context { q[kSecUseAuthenticationContext as String] = context }
        q[kSecReturnData as String] = kCFBooleanTrue as Any
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
