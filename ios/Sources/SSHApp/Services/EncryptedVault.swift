import Foundation
import Security
import SshCoreBridge

/// The synced Keychain items the vault uses (feature 56). Only items marked synchronizable travel
/// through iCloud Keychain; the keys under `com.sshapp.keys` are not among them.
enum VaultAccount {
    /// Tier A: servers, launch profiles, snippets.
    static let servers = "servers"
    /// The passphrase-wrapped vault key (tier B container).
    static let container = "container"
    /// Tier B entries: private keys and secret environment values, sealed with the vault key.
    static let secrets = "secrets"
}

protocol VaultItemStore {
    func load(_ account: String) -> Data?
    func save(_ data: Data, _ account: String) -> Bool
    func delete(_ account: String)
}

struct KeychainVaultStore: VaultItemStore {
    private func base(_ account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "com.sshapp.vault",
         kSecAttrAccount as String: account,
         kSecAttrSynchronizable as String: kCFBooleanTrue as Any]
    }

    func load(_ account: String) -> Data? {
        var q = base(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? Data
    }

    func save(_ data: Data, _ account: String) -> Bool {
        let update = SecItemUpdate(base(account) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if update == errSecSuccess { return true }
        var add = base(account)
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    func delete(_ account: String) {
        SecItemDelete(base(account) as CFDictionary)
    }
}

/// Where this device keeps the unwrapped vault key: the local Keychain, never synchronized.
protocol VaultKeyStorage {
    func load() -> Data?
    func save(_ key: Data) -> Bool
    func delete()
}

struct KeychainVaultKeyStorage: VaultKeyStorage {
    private var base: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "com.sshapp.vaultkey",
         kSecAttrAccount as String: "key",
         kSecAttrSynchronizable as String: kCFBooleanFalse as Any]
    }

    func load() -> Data? {
        var q = base
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? Data
    }

    func save(_ key: Data) -> Bool {
        delete()
        var add = base
        add[kSecValueData as String] = key
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    func delete() {
        SecItemDelete(base as CFDictionary)
    }
}

enum VaultError: LocalizedError, Equatable {
    case mismatch
    case weak
    case wrongPassphrase
    case tooSoon(seconds: Int)
    case unavailable
    case notReady

    var errorDescription: String? {
        switch self {
        case .mismatch: return "The two passphrases are different."
        case .weak: return "Use at least 12 characters."
        case .wrongPassphrase: return "Wrong passphrase, or the vault is damaged."
        case .tooSoon(let s): return "Too many attempts. Try again in \(s) s."
        case .unavailable: return "iCloud Keychain is not available."
        case .notReady: return "The vault is not set up on this device."
        }
    }
}

/// The passphrase vault (feature 56 section 3). The vault key is random; the passphrase only wraps
/// it, so changing the passphrase keeps every sealed entry valid. The wrapped key lives in a synced
/// Keychain item, the unwrapped key only in this device's local Keychain.
@MainActor
final class EncryptedVault: ObservableObject {
    static let shared = EncryptedVault()
    private static let joinedKey = "vault_joined_at"

    enum State: Equatable {
        /// No vault in iCloud.
        case off
        /// A vault exists but this device has not unlocked it (or it was replaced).
        case needsJoin
        /// This device holds a vault key but the vault is gone from iCloud: it was removed on
        /// another device, or someone deleted it. Sync stays paused: an attacker who can delete the
        /// container must not be able to make the app accept unsigned entries.
        case removed
        case ready
    }

    @Published private(set) var state: State = .off
    @Published private(set) var createdAt: Date?
    /// The vault in iCloud is not the one this device joined (reset or replaced elsewhere).
    @Published private(set) var replaced = false

    private let store: VaultItemStore
    private let keys: VaultKeyStorage
    private let defaults: UserDefaults
    private let now: () -> Date
    private static let failuresKey = "vault_unlock_failures"
    private static let blockedKey = "vault_unlock_blocked_until"
    /// Kept in the defaults so relaunching the app does not reset the delay.
    private var failures: Int {
        get { defaults.integer(forKey: Self.failuresKey) }
        set { defaults.set(newValue, forKey: Self.failuresKey) }
    }
    private var blockedUntil: Date {
        get { Date(timeIntervalSince1970: defaults.double(forKey: Self.blockedKey)) }
        set { defaults.set(newValue.timeIntervalSince1970, forKey: Self.blockedKey) }
    }

    init(store: VaultItemStore = KeychainVaultStore(), keys: VaultKeyStorage = KeychainVaultKeyStorage(),
         defaults: UserDefaults = .standard, now: @escaping () -> Date = Date.init) {
        self.store = store
        self.keys = keys
        self.defaults = defaults
        self.now = now
        refresh()
    }

    /// The vault key while this device is unlocked, `nil` otherwise.
    var key: Data? { state == .ready ? keys.load() : nil }

    /// Reads iCloud and the local Keychain again. A vault that another device replaced (different
    /// creation time) drops this device's copy of the old key.
    func refresh() {
        guard let container = store.load(VaultAccount.container), container.count <= 4096,
              let info = try? vaultContainerInfo(container: container) else {
            createdAt = nil
            replaced = false
            state = keys.load() != nil && defaults.object(forKey: Self.joinedKey) != nil ? .removed : .off
            return
        }
        createdAt = Date(timeIntervalSince1970: Double(info.createdAtMs) / 1000)
        let joined = defaults.object(forKey: Self.joinedKey) as? UInt64
        if keys.load() != nil, joined == info.createdAtMs {
            replaced = false
            state = .ready
        } else {
            replaced = keys.load() != nil
            if keys.load() != nil { keys.delete() }
            state = .needsJoin
        }
    }

    func create(passphrase: String, confirm: String) throws {
        guard state == .off else { throw VaultError.notReady }
        let phrase = Data(passphrase.utf8)
        guard passphrasesMatch(a: phrase, b: Data(confirm.utf8)) else { throw VaultError.mismatch }
        guard passphraseStrength(passphrase: phrase).acceptable else { throw VaultError.weak }
        let made = try vaultCreate(passphrase: phrase, createdAtMs: VaultSyncEngine.millis(now()))
        let info = try vaultContainerInfo(container: made.container)
        guard keys.save(made.vaultKey) else { throw VaultError.unavailable }
        guard store.save(made.container, VaultAccount.container) else {
            keys.delete()
            throw VaultError.unavailable
        }
        defaults.set(info.createdAtMs, forKey: Self.joinedKey)
        refresh()
    }

    /// Unlocks the vault another device made. Wrong passphrases are delayed (1 s, doubling to a
    /// minute) so a stolen unlocked phone is not a guessing oracle.
    func join(passphrase: String) throws {
        guard state == .needsJoin, let container = store.load(VaultAccount.container),
              let info = try? vaultContainerInfo(container: container) else { throw VaultError.notReady }
        try checkDelay()
        do {
            let key = try vaultUnlock(container: container, passphrase: Data(passphrase.utf8))
            guard keys.save(key) else { throw VaultError.unavailable }
            failures = 0
            defaults.set(info.createdAtMs, forKey: Self.joinedKey)
            refresh()
        } catch SshCoreError.VaultWrongPassphrase {
            noteFailure()
            throw VaultError.wrongPassphrase
        }
    }

    /// Wraps the same key with a new passphrase; nothing else has to change.
    func changePassphrase(old: String, new: String, confirm: String) throws {
        guard state == .ready, let container = store.load(VaultAccount.container) else { throw VaultError.notReady }
        let phrase = Data(new.utf8)
        guard passphrasesMatch(a: phrase, b: Data(confirm.utf8)) else { throw VaultError.mismatch }
        guard passphraseStrength(passphrase: phrase).acceptable else { throw VaultError.weak }
        try checkDelay()
        do {
            let wrapped = try vaultRewrap(container: container, oldPassphrase: Data(old.utf8), newPassphrase: phrase)
            guard store.save(wrapped, VaultAccount.container) else { throw VaultError.unavailable }
            failures = 0
        } catch SshCoreError.VaultWrongPassphrase {
            noteFailure()
            throw VaultError.wrongPassphrase
        }
    }

    /// Throws the vault away (key, container, sealed entries) and makes a new one with a new key.
    /// Other devices see a different creation time and have to join again: this is how a lost
    /// device is cut off.
    func reset(passphrase: String, confirm: String) throws {
        let phrase = Data(passphrase.utf8)
        guard passphrasesMatch(a: phrase, b: Data(confirm.utf8)) else { throw VaultError.mismatch }
        guard passphraseStrength(passphrase: phrase).acceptable else { throw VaultError.weak }
        turnOff()
        try create(passphrase: passphrase, confirm: confirm)
    }

    /// Drops this device's key after the vault disappeared from iCloud; nothing in iCloud changes.
    func forget() {
        keys.delete()
        defaults.removeObject(forKey: Self.joinedKey)
        refresh()
    }

    /// Deletes the vault from iCloud and this device. Local data stays.
    func turnOff() {
        store.delete(VaultAccount.container)
        store.delete(VaultAccount.secrets)
        keys.delete()
        defaults.removeObject(forKey: Self.joinedKey)
        refresh()
    }

    private func checkDelay() throws {
        let wait = blockedUntil.timeIntervalSince(now())
        if wait > 0 { throw VaultError.tooSoon(seconds: Int(wait.rounded(.up))) }
    }

    private func noteFailure() {
        failures += 1
        blockedUntil = now().addingTimeInterval(min(60, pow(2, Double(failures - 1))))
    }
}
