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

    private init() {}

    public func hasPrivateKey() -> Bool {
        return getPrivateKey() != nil
    }

    public func hasPublicKey() -> Bool {
        return getPublicKey() != nil
    }

    public func savePrivateKey(_ keyPEM: String, overwrite: Bool = false) throws {
        if !overwrite && hasPrivateKey() {
            throw KeychainError.keyAlreadyExists
        }

        guard let data = keyPEM.data(using: .utf8) else {
            throw KeychainError.invalidData
        }

        // Delete any existing key first
        try? deletePrivateKey()

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateKeyAccount,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getPrivateKey() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateKeyAccount,
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

    public func deletePrivateKey() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: privateKeyAccount
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func savePublicKey(_ keyString: String, overwrite: Bool = false) throws {
        if !overwrite && hasPublicKey() {
            throw KeychainError.keyAlreadyExists
        }

        guard let data = keyString.data(using: .utf8) else {
            throw KeychainError.invalidData
        }

        try? deletePublicKey()

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicKeyAccount,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getPublicKey() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicKeyAccount,
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

    public func deletePublicKey() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: publicKeyAccount
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unhandledError(status: status)
        }
    }

    public func getOrDerivePublicKey() -> String? {
        if let key = getPublicKey(), !key.isEmpty {
            return key
        }
        if let privKey = getPrivateKey(), !privKey.isEmpty {
            if let derived = try? derivePublicKey(privateKeyOpenssh: privKey), !derived.isEmpty {
                try? savePublicKey(derived, overwrite: true)
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
