import Foundation
import Security
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

public final class KeychainManager {
    public static let shared = KeychainManager()
    private let serviceName = "com.sshapp.keys"
    private let privateKeyAccount = "ed25519_user_key"
    private let publicKeyAccount = "ed25519_user_public_key"

    /// `nil` is the original single key; every extra key lives under its own account.
    private func privateAccount(_ id: String?) -> String {
        id.map { "ed25519_key_\($0)" } ?? privateKeyAccount
    }
    private func publicAccount(_ id: String?) -> String {
        id.map { "ed25519_key_pub_\($0)" } ?? publicKeyAccount
    }

    private init() {}

    public func hasPrivateKey(id: String? = nil) -> Bool {
        return getPrivateKey(id: id) != nil
    }

    public func hasPublicKey(id: String? = nil) -> Bool {
        return getPublicKey(id: id) != nil
    }

    public func savePrivateKey(_ keyPEM: String, id: String? = nil, overwrite: Bool = false) throws {
        if !overwrite && hasPrivateKey(id: id) {
            throw KeychainError.keyAlreadyExists
        }

        guard let data = keyPEM.data(using: .utf8) else {
            throw KeychainError.invalidData
        }

        // Delete any existing key first
        try? deletePrivateKey(id: id)

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateAccount(id),
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getPrivateKey(id: String? = nil) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateAccount(id),
            kSecReturnData as String: kCFBooleanTrue as Any,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }

        return String(data: data, encoding: .utf8)
    }

    public func deletePrivateKey(id: String? = nil) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateAccount(id)
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func savePublicKey(_ keyString: String, id: String? = nil, overwrite: Bool = false) throws {
        if !overwrite && hasPublicKey(id: id) {
            throw KeychainError.keyAlreadyExists
        }

        guard let data = keyString.data(using: .utf8) else {
            throw KeychainError.invalidData
        }

        try? deletePublicKey(id: id)

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicAccount(id),
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getPublicKey(id: String? = nil) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicAccount(id),
            kSecReturnData as String: kCFBooleanTrue as Any,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }

        return String(data: data, encoding: .utf8)
    }

    public func deletePublicKey(id: String? = nil) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicAccount(id)
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getOrDerivePublicKey(id: String? = nil) -> String? {
        if let key = getPublicKey(id: id), !key.isEmpty {
            return key
        }
        if let privKey = getPrivateKey(id: id), !privKey.isEmpty {
            if let derived = try? derivePublicKey(privateKeyOpenssh: privKey), !derived.isEmpty {
                try? savePublicKey(derived, id: id, overwrite: true)
                return derived
            }
        }
        // Never auto-generate keypairs in the background. Only the user can explicitly generate or regenerate.
        return nil
    }

    public func copyPublicKeyToClipboard(_ publicKey: String) {
        #if canImport(UIKit)
        UIPasteboard.general.string = publicKey
        let generator = UIImpactFeedbackGenerator(style: .medium)
        generator.prepare()
        generator.impactOccurred()
        #elseif canImport(AppKit)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(publicKey, forType: .string)
        #endif
    }
}

public enum KeychainError: LocalizedError {
    case invalidData
    case keyAlreadyExists
    case unhandledError(status: OSStatus)

    public var errorDescription: String? {
        match_error()
    }

    private func match_error() -> String {
        switch self {
        case .invalidData:
            return "Failed to encode private key data"
        case .keyAlreadyExists:
            return "An SSH keypair already exists in Keychain. Overwriting is protected."
        case .unhandledError(let status):
            return "Keychain error code: \(status)"
        }
    }
}


/// A named SSH key. The default key (`id == nil`) is the one created before multiple keys existed.
public struct SSHKeyInfo: Identifiable, Codable, Equatable, Sendable {
    public var id: String
    public var name: String
}

/// Names of the extra keys (not secret); the private halves live only in the Keychain.
@MainActor
public final class SSHKeyStore: ObservableObject {
    public static let shared = SSHKeyStore()
    private static let key = "ssh_extra_keys"
    @Published public private(set) var extra: [SSHKeyInfo] = []

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key),
           let list = try? JSONDecoder().decode([SSHKeyInfo].self, from: data) {
            extra = list
        }
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(extra) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }

    public func name(for id: String?) -> String {
        guard let id else { return "Default key" }
        return extra.first(where: { $0.id == id })?.name ?? "Missing key"
    }

    /// Generates a key under a new name.
    @discardableResult
    public func generate(name: String) throws -> SSHKeyInfo {
        let pair = try generateSshKeypair()
        return try store(name: name, privateKey: pair.privateKeyOpenssh, publicKey: pair.publicKeyOpenssh)
    }

    /// Imports an OpenSSH Ed25519 private key, decrypting it with `passphrase` when it has one. The
    /// key derivation runs off the main thread (a high bcrypt rounds count takes seconds); if the
    /// calling task is cancelled meanwhile, nothing is stored. The Keychain holds the key without
    /// the passphrase, so the passphrase is not kept anywhere.
    @discardableResult
    public func importKey(name: String, privateKey: String, passphrase: Data? = nil) async throws -> SSHKeyInfo {
        let armored = privateKey.trimmingCharacters(in: .whitespacesAndNewlines) + "\n"
        let imported = try await Task.detached(priority: .userInitiated) {
            try importPrivateKey(armoredKey: armored, passphrase: passphrase)
        }.value
        try Task.checkCancellation()
        return try store(name: name, privateKey: imported.privateKeyOpenssh, publicKey: imported.publicKeyOpenssh)
    }

    private func store(name: String, privateKey: String, publicKey: String) throws -> SSHKeyInfo {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let info = SSHKeyInfo(id: UUID().uuidString, name: trimmed.isEmpty ? "Key \(extra.count + 2)" : trimmed)
        try KeychainManager.shared.savePrivateKey(privateKey, id: info.id, overwrite: false)
        try KeychainManager.shared.savePublicKey(publicKey, id: info.id, overwrite: true)
        extra.append(info)
        persist()
        return info
    }

    public func rename(id: String, to name: String) {
        guard let i = extra.firstIndex(where: { $0.id == id }) else { return }
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !n.isEmpty { extra[i].name = n; persist() }
    }

    public func delete(id: String) {
        try? KeychainManager.shared.deletePrivateKey(id: id)
        try? KeychainManager.shared.deletePublicKey(id: id)
        extra.removeAll { $0.id == id }
        persist()
    }
}
