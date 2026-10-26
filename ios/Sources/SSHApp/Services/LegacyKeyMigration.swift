import Foundation
import Security
import SshCoreBridge

/// One-time step for the build that moved the private keys into the Rust core. Keys written by the
/// older Swift code sit in the same Keychain items the core reads, so nothing is copied: this checks
/// that the core can read and parse each one, then removes the separate public-key items the old
/// code kept (the core derives the public half on demand). Only the owner's device ever had the old
/// layout; delete this file together with its call in `SSHApp.swift` after the first launch.
enum LegacyKeyMigration {
    private static let doneKey = "legacy_key_migration_done"

    /// `report` is a one-line outcome for the log.
    @discardableResult
    static func runOnce() -> String? {
        guard !UserDefaults.standard.bool(forKey: doneKey) else { return nil }
        var checked = 0, unreadable: [String] = []
        let ids: [String?] = [nil] + SSHKeyStoreSnapshot.ids()
        for id in ids {
            guard keyExists(key: KeyRef(id: id, guarded: false)) else { continue }
            if let info = try? keyInfo(key: KeyRef(id: id, guarded: false)), info != nil { checked += 1 } else { unreadable.append(id ?? "default") }
        }
        guard unreadable.isEmpty else {
            let line = "Key migration: \(checked) readable, unreadable: \(unreadable.joined(separator: ","))"
            print("[Keys] \(line)")
            return line
        }
        for account in ["ed25519_user_public_key"] + ids.compactMap({ $0 }).map({ "ed25519_key_pub_\($0)" }) {
            SecItemDelete([
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: "com.sshapp.keys",
                kSecAttrAccount as String: account
            ] as CFDictionary)
        }
        UserDefaults.standard.set(true, forKey: doneKey)
        let line = "Key migration: \(checked) keys readable by the core, old public-key items removed"
        print("[Keys] \(line)")
        return line
    }
}

/// The ids of the extra keys, read without the main-actor store.
private enum SSHKeyStoreSnapshot {
    static func ids() -> [String] {
        guard let data = UserDefaults.standard.data(forKey: "ssh_extra_keys"),
              let list = try? JSONDecoder().decode([SSHKeyInfo].self, from: data) else { return [] }
        return list.map(\.id)
    }
}
