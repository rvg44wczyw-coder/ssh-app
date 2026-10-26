import Foundation
import Security
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The app's view of the SSH keys. The private halves live in the Rust core (`key_store`); this type
/// only names them and never sees their text.
public final class KeychainManager {
    public static let shared = KeychainManager()
    private init() {}

    /// `nil` is the original single key; every extra key has its own id.
    public func keyRef(_ id: String?, guarded: Bool = false) -> KeyRef {
        KeyRef(id: id, guarded: guarded)
    }

    public func hasPrivateKey(id: String? = nil) -> Bool {
        keyExists(key: keyRef(id))
    }

    /// The public line of a stored key, derived by the core on demand.
    public func getOrDerivePublicKey(id: String? = nil) -> String? {
        (try? keyInfo(key: keyRef(id)))??.publicKeyOpenssh
    }

    /// Never generates in the background: only the owner explicitly generates or regenerates.
    @discardableResult
    public func generateKey(id: String? = nil, overwrite: Bool) throws -> KeyInfo {
        try keyGenerate(key: keyRef(id), overwrite: overwrite)
    }

    public func deleteKey(id: String? = nil) throws {
        try keyDelete(key: keyRef(id))
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

    /// Generates a key under a new name, inside the core.
    @discardableResult
    public func generate(name: String) throws -> SSHKeyInfo {
        let info = newInfo(name: name)
        try keyGenerate(key: KeyRef(id: info.id, guarded: false), overwrite: false)
        extra.append(info)
        persist()
        return info
    }

    /// Imports an OpenSSH Ed25519 key file given as bytes, decrypting it with `passphrase` when it
    /// has one. The key derivation runs off the main thread (a high bcrypt rounds count takes
    /// seconds); if the calling task is cancelled meanwhile, nothing is stored. Both inputs are
    /// wiped when the core is done, and the passphrase is not kept anywhere.
    @discardableResult
    public func importKey(name: String, privateKey: Data, passphrase: Data? = nil) async throws -> SSHKeyInfo {
        let info = newInfo(name: name)
        var armored = privateKey
        defer { armored.resetBytes(in: 0..<armored.count) }
        let bytes = armored, pass = passphrase
        try await Task.detached(priority: .userInitiated) {
            _ = try keyImport(key: KeyRef(id: info.id, guarded: false), armored: bytes, passphrase: pass, overwrite: false)
        }.value
        try Task.checkCancellation()
        extra.append(info)
        persist()
        return info
    }

    private func newInfo(name: String) -> SSHKeyInfo {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return SSHKeyInfo(id: UUID().uuidString, name: trimmed.isEmpty ? "Key \(extra.count + 2)" : trimmed)
    }

    /// Records a key the core already stored (arrived through the encrypted vault or a backup).
    func adopt(id: String?, name: String) {
        guard let id else { return }
        if let i = extra.firstIndex(where: { $0.id == id }) {
            extra[i].name = name
        } else {
            extra.append(SSHKeyInfo(id: id, name: name))
        }
        persist()
    }

    public func rename(id: String, to name: String) {
        guard let i = extra.firstIndex(where: { $0.id == id }) else { return }
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !n.isEmpty { extra[i].name = n; persist() }
    }

    public func delete(id: String) {
        try? KeychainManager.shared.deleteKey(id: id)
        extra.removeAll { $0.id == id }
        persist()
    }
}
