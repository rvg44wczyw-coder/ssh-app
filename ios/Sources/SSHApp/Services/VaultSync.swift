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

/// Keeps the server list the same on every device of the same iCloud account. The phone itself
/// does not touch the network: the Keychain item is moved by the system.
@MainActor
final class VaultSync: ObservableObject {
    static let shared = VaultSync()
    /// The master switch (feature 56): off by default, nothing touches iCloud while it is off.
    static let enabledKey = "vault_sync_enabled"
    static let noticeKey = "vault_sync_notice"
    private static let migratedKey = "vault_sync_v2_migrated"
    static let serversKey = "vault_sync_servers"
    static let fingerprintsKey = "vault_sync_fingerprints"
    private static let tombstonesKey = "vault_tombstones"
    /// A blob larger than this is not read: the item is shared with other devices.

    @Published private(set) var lastSync: Date?
    @Published private(set) var fingerprintConflicts: [String] = []
    @Published private(set) var lastError: String?
    /// Entries in iCloud that were ignored because the vault key did not sign them.
    @Published private(set) var rejectedEntries = 0
    @Published private(set) var secretsRejected = 0

    var store: VaultItemStore = KeychainVaultStore()
    var vault: EncryptedVault = .shared
    private weak var manager: ServerProfilesManager?
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var enabled: Bool { defaults.bool(forKey: Self.enabledKey) }
    var serversEnabled: Bool { defaults.object(forKey: Self.serversKey) as? Bool ?? true }
    var fingerprintsEnabled: Bool { defaults.bool(forKey: Self.fingerprintsKey) }

    /// The update that introduced the master switch turns sync off for everyone (the user decides);
    /// whoever had it running gets a notice in Settings until they open the sync page.
    func migrateIfNeeded() {
        guard !defaults.bool(forKey: Self.migratedKey) else { return }
        defaults.set(true, forKey: Self.migratedKey)
        let used = store.load(VaultAccount.servers) != nil || defaults.object(forKey: "vault_tombstones") != nil
            || defaults.object(forKey: "vault_state_profiles") != nil || defaults.object(forKey: "vault_state_snippets") != nil
        defaults.set(false, forKey: Self.enabledKey)
        if used { defaults.set(true, forKey: Self.noticeKey) }
    }

    func attach(_ manager: ServerProfilesManager) {
        migrateIfNeeded()
        self.manager = manager
    }

    func recordDeletion(of id: UUID) {
        var stones = tombstones
        stones[id.uuidString] = VaultSyncEngine.millis(Date())
        tombstones = stones
    }

    /// Merges the vault with the local lists and writes the result to both: servers, launch
    /// profiles and snippets, each switched on or off on its own.
    func sync(now: Date = Date()) {
        guard enabled, let manager else { return }
        vault.refresh()
        switch vault.state {
        case .needsJoin:
            lastError = vault.replaced
                ? "The vault was reset or replaced. Unlock it with its passphrase, or reset it."
                : "Enter the vault passphrase on this device to continue syncing."
            return
        case .removed:
            lastError = "The vault is gone from iCloud. Sync is paused until you set it up again or forget it on this device."
            return
        case .off, .ready:
            break
        }
        let vaultKey = vault.key
        let remote = readRemote(key: vaultKey)
        let isOther: (VaultEntry) -> Bool = { e in [SyncedProfile.prefix, SyncedSnippet.prefix, SyncedQuickCommand.prefix, SyncedAppearance.prefix].contains { e.id.hasPrefix($0) } }
        var entries: [VaultEntry] = []

        let serverRemote = remote.filter { !isOther($0) }
        if serversEnabled {
            let outcome = VaultSyncEngine.reconcile(
                local: manager.profiles, tombstones: tombstones, remote: serverRemote,
                syncFingerprints: fingerprintsEnabled, now: now)
            if outcome.profilesChanged { manager.applySynced(outcome.profiles) }
            tombstones = outcome.tombstones
            fingerprintConflicts = outcome.fingerprintConflicts
            entries += VaultSyncEngine.entries(for: outcome.profiles, tombstones: outcome.tombstones, syncFingerprints: fingerprintsEnabled)
        } else {
            entries += serverRemote
        }

        if profilesEnabled, let agents = agentProfiles {
            let outcome = VaultCollectionEngine.reconcile(
                prefix: SyncedProfile.prefix, local: SyncedProfile.outgoing(agents.profiles), remote: remote,
                state: collectionState(Self.profileStateKey), now: now,
                accept: { SyncedProfile.decode($0) != nil })
            if !outcome.upserts.isEmpty || !outcome.deletes.isEmpty {
                var list = agents.profiles
                list.removeAll { outcome.deletes.contains($0.id) }
                for item in outcome.upserts {
                    guard let incoming = SyncedProfile.decode(item) else { continue }
                    let i = list.firstIndex { $0.id == incoming.id }
                    let merged = SyncedProfile.merged(incoming, into: i.map { list[$0] })
                    if let i { list[i] = merged } else { list.append(merged) }
                }
                agents.applySynced(list)
            }
            setCollectionState(outcome.state, Self.profileStateKey)
            entries += outcome.entries
        } else {
            entries += remote.filter { $0.id.hasPrefix(SyncedProfile.prefix) }
        }

        if snippetsEnabled {
            let local = storedSnippets()
            let outcome = VaultCollectionEngine.reconcile(
                prefix: SyncedSnippet.prefix,
                local: SyncedSnippet.outgoing(local.map { SyncedSnippet(id: $0.id, title: $0.title, command: $0.command, category: $0.category, tags: $0.tags) }),
                remote: remote, state: collectionState(Self.snippetStateKey), now: now,
                accept: { SyncedSnippet.decode($0) != nil })
            if !outcome.upserts.isEmpty || !outcome.deletes.isEmpty {
                var list = local
                list.removeAll { outcome.deletes.contains($0.id) }
                for item in outcome.upserts {
                    guard let s = SyncedSnippet.decode(item) else { continue }
                    let stored = StoredSnippet(id: s.id, title: s.title, command: s.command, category: s.category, tags: s.tags,
                                               useCount: list.first { $0.id == s.id }?.useCount ?? 5)
                    if let i = list.firstIndex(where: { $0.id == s.id }) { list[i] = stored } else { list.insert(stored, at: 0) }
                }
                storeSnippets(list)
            }
            setCollectionState(outcome.state, Self.snippetStateKey)
            entries += outcome.entries
        } else {
            entries += remote.filter { $0.id.hasPrefix(SyncedSnippet.prefix) }
        }

        entries += syncCollection(
            prefix: SyncedQuickCommand.prefix, enabled: quickCommandsEnabled, stateKey: Self.quickStateKey,
            local: SyncedQuickCommand.outgoing(QuickCommandStore.shared.commands), remote: remote, now: now,
            accept: { SyncedQuickCommand.decode($0) != nil },
            apply: { upserts, deletes in
                let store = QuickCommandStore.shared
                var list = store.commands
                list.removeAll { c in deletes.contains(SyncedQuickCommand(c).id) }
                for item in upserts {
                    guard let s = SyncedQuickCommand.decode(item), !list.contains(where: { SyncedQuickCommand($0).id == s.id }) else { continue }
                    list.append(s.command)
                }
                if list != store.commands { store.commands = list }
            })
        entries += syncCollection(
            prefix: SyncedAppearance.prefix, enabled: appearanceEnabled, stateKey: Self.appearanceStateKey,
            local: SyncedAppearance.outgoing(defaults: defaults), remote: remote, now: now,
            accept: { SyncedAppearance.decode($0) != nil },
            apply: { upserts, _ in
                // A deletion never resets the colour on this device.
                if let item = upserts.last, let accent = SyncedAppearance.decode(item) {
                    defaults.set(accent, forKey: AppAccent.storageKey)
                }
            })

        if entries.sorted(by: { $0.id < $1.id }) != remote.sorted(by: { $0.id < $1.id }) || rejectedEntries > 0 {
            writeRemote(entries, key: vaultKey)
        } else {
            lastError = nil
        }
        if let vaultKey { syncSecrets(key: vaultKey, now: now) }
        lastSync = now
    }

    // MARK: Launch profiles and snippets

    static let profilesKey = "vault_sync_profiles"
    static let snippetsKey = "vault_sync_snippets"
    private static let profileStateKey = "vault_state_profiles"
    private static let snippetStateKey = "vault_state_snippets"
    static let snippetsStorageKey = "ssh_app.custom_snippets.v1"

    static let quickCommandsKey = "vault_sync_quick_commands"
    static let appearanceKey = "vault_sync_appearance"
    private static let quickStateKey = "vault_state_quick_commands"
    private static let appearanceStateKey = "vault_state_appearance"
    var quickCommandsEnabled: Bool { defaults.object(forKey: Self.quickCommandsKey) as? Bool ?? true }
    var appearanceEnabled: Bool { defaults.object(forKey: Self.appearanceKey) as? Bool ?? true }

    /// One synced list: merged with the vault, changes applied locally, the entries to write back.
    /// Switched off, the list is left in the vault as it is.
    private func syncCollection(
        prefix: String, enabled: Bool, stateKey: String, local: [CollectionItem], remote: [VaultEntry], now: Date,
        accept: (CollectionItem) -> Bool, apply: ([CollectionItem], [String]) -> Void
    ) -> [VaultEntry] {
        guard enabled else { return remote.filter { $0.id.hasPrefix(prefix) } }
        let outcome = VaultCollectionEngine.reconcile(
            prefix: prefix, local: local, remote: remote, state: collectionState(stateKey), now: now, accept: accept)
        if !outcome.upserts.isEmpty || !outcome.deletes.isEmpty { apply(outcome.upserts, outcome.deletes) }
        setCollectionState(outcome.state, stateKey)
        return outcome.entries
    }

    var profilesEnabled: Bool { defaults.object(forKey: Self.profilesKey) as? Bool ?? true }
    var snippetsEnabled: Bool { defaults.object(forKey: Self.snippetsKey) as? Bool ?? true }
    private weak var agentProfiles: AgentProfilesManager?

    func attach(agentProfiles: AgentProfilesManager) {
        self.agentProfiles = agentProfiles
        sync()
    }

    /// The same record the snippets sheet keeps.
    struct StoredSnippet: Codable, Equatable {
        var id: String
        var title: String
        var command: String
        var category: String
        var tags: [String]
        var useCount: UInt64
    }

    private func storedSnippets() -> [StoredSnippet] {
        guard let data = defaults.data(forKey: Self.snippetsStorageKey) else { return [] }
        return (try? JSONDecoder().decode([StoredSnippet].self, from: data)) ?? []
    }

    private func storeSnippets(_ list: [StoredSnippet]) {
        if let data = try? JSONEncoder().encode(list) { defaults.set(data, forKey: Self.snippetsStorageKey) }
    }

    private func collectionState(_ key: String) -> CollectionState {
        guard let data = defaults.data(forKey: key), let st = try? JSONDecoder().decode(CollectionState.self, from: data) else {
            return CollectionState()
        }
        return st
    }

    private func setCollectionState(_ state: CollectionState, _ key: String) {
        if let data = try? JSONEncoder().encode(state) { defaults.set(data, forKey: key) }
    }

    // MARK: Secrets (tier B)

    static let secretKeysKey = "vault_sync_secret_keys"
    static let secretEnvKey = "vault_sync_secret_env"
    private static let secretStateKey = "vault_state_secrets"

    var secretKeysEnabled: Bool { defaults.bool(forKey: Self.secretKeysKey) }
    var secretEnvEnabled: Bool { defaults.bool(forKey: Self.secretEnvKey) }

    private var secretState: SecretState {
        get {
            guard let data = defaults.data(forKey: Self.secretStateKey),
                  let st = try? JSONDecoder().decode(SecretState.self, from: data) else { return SecretState() }
            return st
        }
        set { if let data = try? JSONEncoder().encode(newValue) { defaults.set(data, forKey: Self.secretStateKey) } }
    }

    /// What this device could share, for the lists in the sync page.
    func secretOffers() -> (keys: [SecretItem], env: [SecretItem]) {
        (VaultSecretItems.keyItems(default: KeychainManager.shared.hasPrivateKey(), extra: SSHKeyStore.shared.extra),
         VaultSecretItems.envItems(agentProfiles?.profiles ?? []))
    }

    func secretEnabled(_ id: String) -> Bool { secretState.enabled.contains(id) }

    /// Turning an item off removes its copy in iCloud on the next sync; other devices keep theirs.
    func setSecret(_ id: String, enabled on: Bool) {
        var st = secretState
        if on {
            st.enabled.insert(id)
            st.dismissed.remove(id)
            st.drop.remove(id)
        } else {
            st.enabled.remove(id)
            st.dismissed.insert(id)
            st.drop.insert(id)
            st.payloads[id] = nil
            st.stamps[id] = nil
        }
        secretState = st
        sync()
    }

    private func syncSecrets(key: Data, now: Date) {
        let keysOn = secretKeysEnabled, envOn = secretEnvEnabled
        guard keysOn || envOn, let agents = agentProfiles else { return }
        let stored = store.load(VaultAccount.secrets)
        let opened = stored.map { VaultWire.decodeSealed($0, key: key) } ?? (entries: [], rejected: 0)
        secretsRejected = opened.rejected
        let offers = secretOffers()
        let known = Set(agents.profiles.map(\.id))
        let hasDefault = KeychainManager.shared.hasPrivateKey()
        let outcome = SecretSyncEngine.reconcile(
            local: (keysOn ? offers.keys : []) + (envOn ? offers.env : []),
            remote: opened.entries, state: secretState, now: now,
            inScope: { ($0.hasPrefix(SecretIDs.keyPrefix) && keysOn) || ($0.hasPrefix(SecretIDs.envPrefix) && envOn) },
            accept: { VaultSecretItems.accepts($0, knownProfiles: known, hasDefaultKey: hasDefault) })
        for item in outcome.upserts {
            VaultSecretItems.apply(item, agents: agents, keys: SSHKeyStore.shared)
        }
        secretState = outcome.state
        if outcome.remote != opened.entries.sorted(by: { $0.id < $1.id }) || opened.rejected > 0 {
            guard let data = VaultWire.encodeSealed(outcome.remote, key: key) else {
                lastError = "The secrets are too large to sync"
                return
            }
            if !store.save(data, VaultAccount.secrets) { lastError = "iCloud Keychain is not available" }
        }
    }

    /// What the next sync would carry, for "What will be sent".
    func outgoingSummary() -> [(title: String, count: Int)] {
        var out: [(String, Int)] = []
        if serversEnabled { out.append(("Servers", manager?.profiles.count ?? 0)) }
        if profilesEnabled { out.append(("Launch profiles", SyncedProfile.outgoing(agentProfiles?.profiles ?? []).count)) }
        if snippetsEnabled { out.append(("Snippets", storedSnippets().count)) }
        if quickCommandsEnabled { out.append(("Quick commands", SyncedQuickCommand.outgoing(QuickCommandStore.shared.commands).count)) }
        if appearanceEnabled { out.append(("Accent colour", SyncedAppearance.outgoing(defaults: defaults).count)) }
        let st = secretState
        if secretKeysEnabled { out.append(("Private keys", st.enabled.filter { $0.hasPrefix(SecretIDs.keyPrefix) }.count)) }
        if secretEnvEnabled { out.append(("Secret values", st.enabled.filter { $0.hasPrefix(SecretIDs.envPrefix) }.count)) }
        return out
    }

    /// Deletes everything this app put in iCloud (the lists and the sealed secrets) and switches
    /// sync off. The vault itself stays until it is turned off; local data is untouched.
    func removeFromICloud() {
        store.delete(VaultAccount.servers)
        store.delete(VaultAccount.secrets)
        defaults.set(false, forKey: Self.enabledKey)
        lastSync = nil
        lastError = nil
    }

    private func readRemote(key: Data?) -> [VaultEntry] {
        guard let data = store.load(VaultAccount.servers) else { rejectedEntries = 0; return [] }
        let result = VaultWire.decodeSigned(data, key: key)
        rejectedEntries = result.rejected
        return result.entries
    }

    private func writeRemote(_ entries: [VaultEntry], key: Data?) {
        guard let data = VaultWire.encodeSigned(entries, key: key) else {
            lastError = "The synced list is too large"
            return
        }
        lastError = store.save(data, VaultAccount.servers) ? nil : "iCloud Keychain is not available"
        if lastError == nil { rejectedEntries = 0 }
    }

    private var tombstones: [String: UInt64] {
        get { (defaults.dictionary(forKey: Self.tombstonesKey) as? [String: NSNumber])?.mapValues { $0.uint64Value } ?? [:] }
        set { defaults.set(newValue.mapValues { NSNumber(value: $0) }, forKey: Self.tombstonesKey) }
    }
}
