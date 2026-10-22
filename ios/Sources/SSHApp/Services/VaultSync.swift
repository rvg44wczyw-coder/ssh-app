import Foundation
import Security
import SshCoreBridge

/// What travels for one server. Never a key, never the device's own key choice (`keyId`): each
/// device signs in with its own key (feature 43). The pinned host fingerprint only with its toggle.
struct SyncedServer: Codable, Equatable {
    var name: String
    var host: String
    var port: UInt16
    var username: String
    var fingerprint: String?

    /// The other side is not trusted to be well-formed: a value that could not be a real name,
    /// address or login is dropped rather than turned into a profile.
    var isSane: Bool {
        func clean(_ s: String, max: Int) -> Bool {
            !s.isEmpty && s.count <= max && !s.hasPrefix("-")
                && !s.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) || CharacterSet.whitespacesAndNewlines.contains($0) }
        }
        return clean(host, max: 253) && clean(username, max: 64) && port > 0
            && !name.isEmpty && name.count <= 100
            && !name.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
    }
}

/// The pure part of the sync: profiles in, merged profiles out. The side that wins is decided by
/// the Rust core (`mergeVault`); this only translates to and from `ServerProfile`.
enum VaultSyncEngine {
    struct Outcome {
        var profiles: [ServerProfile]
        var entries: [VaultEntry]
        var tombstones: [String: UInt64]
        /// Servers whose pinned fingerprint differs from the one the other device pinned.
        var fingerprintConflicts: [String]
        var profilesChanged: Bool
    }

    static func millis(_ date: Date?) -> UInt64 {
        guard let date else { return 1 }
        return UInt64(max(0, date.timeIntervalSince1970) * 1000)
    }

    static func entries(for profiles: [ServerProfile], tombstones: [String: UInt64], syncFingerprints: Bool) -> [VaultEntry] {
        var out = profiles.compactMap { p -> VaultEntry? in
            let body = SyncedServer(name: p.name, host: p.host, port: p.port, username: p.username,
                                    fingerprint: syncFingerprints ? p.hostKeyFingerprint : nil)
            guard let data = try? JSONEncoder().encode(body), let json = String(data: data, encoding: .utf8) else { return nil }
            return VaultEntry(id: p.id.uuidString, updatedAtMs: millis(p.updatedAt), deleted: false, payload: json)
        }
        out += tombstones.map { VaultEntry(id: $0.key, updatedAtMs: $0.value, deleted: true, payload: "") }
        return out
    }

    static func reconcile(
        local: [ServerProfile], tombstones: [String: UInt64], remote: [VaultEntry],
        syncFingerprints: Bool, now: Date
    ) -> Outcome {
        let live = Set(local.map { $0.id.uuidString })
        // A server that exists locally is not also a tombstone (it was re-added after a delete).
        let stones = tombstones.filter { !live.contains($0.key) }
        let merge = mergeVault(local: entries(for: local, tombstones: stones, syncFingerprints: syncFingerprints),
                               remote: remote, nowMs: millis(now))
        let taken = Set(merge.takenFromRemote)
        var profiles = local
        var conflicts: [String] = []
        var changed = false

        for entry in merge.merged where taken.contains(entry.id) {
            guard let id = UUID(uuidString: entry.id) else { continue }
            let index = profiles.firstIndex { $0.id == id }
            if entry.deleted {
                if let index { profiles.remove(at: index); changed = true }
                continue
            }
            guard let body = try? JSONDecoder().decode(SyncedServer.self, from: Data(entry.payload.utf8)),
                  body.isSane else { continue }
            let stamp = Date(timeIntervalSince1970: Double(entry.updatedAtMs) / 1000)
            if let index {
                profiles[index].name = body.name
                profiles[index].host = body.host
                profiles[index].port = body.port
                profiles[index].username = body.username
                profiles[index].updatedAt = stamp
                if syncFingerprints, let theirs = body.fingerprint {
                    if profiles[index].hostKeyFingerprint == nil {
                        profiles[index].hostKeyFingerprint = theirs
                    } else if profiles[index].hostKeyFingerprint != theirs {
                        conflicts.append(profiles[index].name)
                    }
                }
            } else {
                profiles.append(ServerProfile(
                    id: id, name: body.name, host: body.host, port: body.port, username: body.username,
                    hostKeyFingerprint: syncFingerprints ? body.fingerprint : nil, updatedAt: stamp))
            }
            changed = true
        }
        let kept = Dictionary(uniqueKeysWithValues: merge.merged.filter(\.deleted).map { ($0.id, $0.updatedAtMs) })
        return Outcome(profiles: profiles, entries: merge.merged, tombstones: kept,
                       fingerprintConflicts: conflicts, profilesChanged: changed)
    }
}

/// Where the merged list lives: one synchronizable Keychain item, which iCloud Keychain carries
/// end to end encrypted. Items without `kSecAttrSynchronizable` (the keys) never match it.
protocol VaultStore {
    func load() -> Data?
    func save(_ data: Data) -> Bool
}

struct KeychainVaultStore: VaultStore {
    private var base: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "com.sshapp.vault",
         kSecAttrAccount as String: "servers",
         kSecAttrSynchronizable as String: kCFBooleanTrue as Any]
    }

    func load() -> Data? {
        var q = base
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? Data
    }

    func save(_ data: Data) -> Bool {
        let update = SecItemUpdate(base as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if update == errSecSuccess { return true }
        var add = base
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }
}

/// Keeps the server list the same on every device of the same iCloud account. The phone itself
/// does not touch the network: the Keychain item is moved by the system.
@MainActor
final class VaultSync: ObservableObject {
    static let shared = VaultSync()
    static let serversKey = "vault_sync_servers"
    static let fingerprintsKey = "vault_sync_fingerprints"
    private static let tombstonesKey = "vault_tombstones"
    /// A blob larger than this is not read: the item is shared with other devices.
    private static let maxBlobBytes = 512 * 1024

    @Published private(set) var lastSync: Date?
    @Published private(set) var fingerprintConflicts: [String] = []
    @Published private(set) var lastError: String?

    var store: VaultStore = KeychainVaultStore()
    private weak var manager: ServerProfilesManager?
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var serversEnabled: Bool { defaults.object(forKey: Self.serversKey) as? Bool ?? true }
    var fingerprintsEnabled: Bool { defaults.bool(forKey: Self.fingerprintsKey) }

    func attach(_ manager: ServerProfilesManager) {
        self.manager = manager
    }

    func recordDeletion(of id: UUID) {
        var stones = tombstones
        stones[id.uuidString] = VaultSyncEngine.millis(Date())
        tombstones = stones
    }

    /// Merges the vault with the local list and writes the result to both.
    func sync(now: Date = Date()) {
        guard serversEnabled, let manager else { return }
        let remote = readRemote()
        let outcome = VaultSyncEngine.reconcile(
            local: manager.profiles, tombstones: tombstones, remote: remote,
            syncFingerprints: fingerprintsEnabled, now: now)
        if outcome.profilesChanged { manager.applySynced(outcome.profiles) }
        tombstones = outcome.tombstones
        fingerprintConflicts = outcome.fingerprintConflicts
        let own = VaultSyncEngine.entries(for: outcome.profiles, tombstones: outcome.tombstones, syncFingerprints: fingerprintsEnabled)
        if own.sorted(by: { $0.id < $1.id }) != remote.sorted(by: { $0.id < $1.id }) {
            writeRemote(own)
        }
        lastSync = now
    }

    private func readRemote() -> [VaultEntry] {
        guard let data = store.load(), data.count <= Self.maxBlobBytes,
              let wire = try? JSONDecoder().decode([WireEntry].self, from: data) else { return [] }
        return wire.map(\.entry)
    }

    private func writeRemote(_ entries: [VaultEntry]) {
        guard let data = try? JSONEncoder().encode(entries.map(WireEntry.init)), data.count <= Self.maxBlobBytes else {
            lastError = "The server list is too large to sync"
            return
        }
        lastError = store.save(data) ? nil : "iCloud Keychain is not available"
    }

    private var tombstones: [String: UInt64] {
        get { (defaults.dictionary(forKey: Self.tombstonesKey) as? [String: NSNumber])?.mapValues { $0.uint64Value } ?? [:] }
        set { defaults.set(newValue.mapValues { NSNumber(value: $0) }, forKey: Self.tombstonesKey) }
    }

    private struct WireEntry: Codable {
        var id: String
        var at: UInt64
        var deleted: Bool
        var payload: String
        init(_ e: VaultEntry) { id = e.id; at = e.updatedAtMs; deleted = e.deleted; payload = e.payload }
        var entry: VaultEntry { VaultEntry(id: id, updatedAtMs: at, deleted: deleted, payload: payload) }
    }
}
