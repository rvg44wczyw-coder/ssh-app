import CryptoKit
import Foundation
import SshCoreBridge

/// Tier B of the vault (feature 56): private SSH keys and secret environment values, chosen one by
/// one, sealed with the vault key before they reach iCloud. Nothing here is applied without being
/// validated, and nothing here ever deletes data on another device: turning an item off removes
/// only its copy in iCloud.
struct SecretItem: Equatable {
    var id: String
    /// Shown in the list; not synced.
    var label: String
    /// JSON of `SyncedKey` or `SyncedEnv`.
    var payload: String
}

struct SecretState: Codable, Equatable {
    /// Items the user turned on (or that arrived from another device).
    var enabled: Set<String> = []
    /// Items turned off or deleted here; they are not imported again.
    var dismissed: Set<String> = []
    /// Ids to remove from iCloud on the next sync.
    var drop: Set<String> = []
    var stamps: [String: UInt64] = [:]
    var payloads: [String: String] = [:]
}

/// A key in the sync payload. The private half is `sealed`: the core's ChaCha20-Poly1305 blob under
/// the vault key (base64 of bytes), never the key text.
struct SyncedKey: Codable, Equatable {
    var name: String
    var sealed: String
    var publicKey: String
}

struct SyncedEnv: Codable, Equatable {
    var profile: String
    var name: String
    var value: String
}

enum SecretIDs {
    static let keyPrefix = "k_"
    static let envPrefix = "e_"

    /// The original single key has no id of its own.
    static func key(_ keyId: String?) -> String { keyPrefix + (keyId ?? "default") }

    static func keyId(from id: String) -> String? {
        guard id.hasPrefix(keyPrefix) else { return nil }
        let rest = String(id.dropFirst(keyPrefix.count))
        return rest == "default" ? nil : rest
    }

    static func env(profile: String, name: String) -> String {
        let digest = SHA256.hash(data: Data("\(profile)|\(name)".utf8))
        return envPrefix + digest.map { String(format: "%02x", $0) }.joined().prefix(32)
    }
}

enum SecretSyncEngine {
    struct Outcome: Equatable {
        /// Items to create or replace on this device (already accepted).
        var upserts: [SecretItem]
        /// What iCloud should hold after this sync.
        var remote: [VaultEntry]
        var state: SecretState
    }

    /// Per enabled id the later edit wins (a local edit is noticed by a changed payload). An id that
    /// only exists in iCloud is imported unless it was dismissed here. An enabled id that vanished
    /// locally is dismissed and removed from iCloud: deleting a key here removes its shared copy.
    /// Ids outside `inScope` (a category that is switched off) are left exactly as they are.
    static func reconcile(
        local: [SecretItem], remote: [VaultEntry], state: SecretState, now: Date,
        inScope: (String) -> Bool = { _ in true }, accept: (SecretItem) -> Bool
    ) -> Outcome {
        let nowMs = VaultSyncEngine.millis(now)
        var st = state
        var held: [String: VaultEntry] = [:]
        for r in remote where !r.deleted && !st.drop.contains(r.id) { held[r.id] = r }
        let localIds = Set(local.map(\.id))
        var upserts: [SecretItem] = []

        for item in local where st.enabled.contains(item.id) && inScope(item.id) {
            if st.payloads[item.id] != item.payload {
                st.payloads[item.id] = item.payload
                st.stamps[item.id] = nowMs
            }
            let stamp = st.stamps[item.id] ?? nowMs
            if let theirs = held[item.id], theirs.payload != item.payload, theirs.updatedAtMs > stamp {
                let incoming = SecretItem(id: item.id, label: item.label, payload: theirs.payload)
                if accept(incoming) {
                    upserts.append(incoming)
                    st.payloads[item.id] = theirs.payload
                    st.stamps[item.id] = theirs.updatedAtMs
                    continue
                }
            }
            held[item.id] = VaultEntry(id: item.id, updatedAtMs: stamp, deleted: false, payload: item.payload)
        }

        for id in st.enabled where !localIds.contains(id) && inScope(id) {
            st.enabled.remove(id)
            st.dismissed.insert(id)
            st.payloads[id] = nil
            st.stamps[id] = nil
            held[id] = nil
        }

        for (id, entry) in held.sorted(by: { $0.key < $1.key }) where inScope(id) && !localIds.contains(id) && !st.dismissed.contains(id) {
            let incoming = SecretItem(id: id, label: "", payload: entry.payload)
            guard accept(incoming) else { continue }
            upserts.append(incoming)
            st.enabled.insert(id)
            st.payloads[id] = entry.payload
            st.stamps[id] = entry.updatedAtMs
        }

        st.drop = []
        return Outcome(upserts: upserts, remote: held.values.sorted { $0.id < $1.id }, state: st)
    }
}

/// The wire form of tier B and of the signed tier A list; pure so it can be tested.
enum VaultWire {
    struct SealedEntry: Codable { var id: String; var sealed: Data }
    struct SignedEntry: Codable {
        var id: String
        var at: UInt64
        var deleted: Bool
        var payload: String
        var mac: Data?
    }

    static let maxBlobBytes = 2 * 1024 * 1024

    static func encodeSigned(_ entries: [VaultEntry], key: Data?) -> Data? {
        let wire = entries.map { e -> SignedEntry in
            let mac = key.flatMap { try? entrySign(vaultKey: $0, entry: e) }
            return SignedEntry(id: e.id, at: e.updatedAtMs, deleted: e.deleted, payload: e.payload, mac: mac)
        }
        guard let data = try? JSONEncoder().encode(wire), data.count <= maxBlobBytes else { return nil }
        return data
    }

    /// With a key, an entry without a valid MAC is dropped and counted: someone without the vault
    /// key wrote it. Without a key nothing is checked (no vault was made).
    static func decodeSigned(_ data: Data, key: Data?) -> (entries: [VaultEntry], rejected: Int) {
        guard data.count <= maxBlobBytes, let wire = try? JSONDecoder().decode([SignedEntry].self, from: data) else {
            return ([], 0)
        }
        var out: [VaultEntry] = []
        var rejected = 0
        for w in wire {
            let entry = VaultEntry(id: w.id, updatedAtMs: w.at, deleted: w.deleted, payload: w.payload)
            if let key {
                guard let mac = w.mac, entryVerify(vaultKey: key, entry: entry, mac: mac) else { rejected += 1; continue }
            }
            out.append(entry)
        }
        return (out, rejected)
    }

    static func encodeSealed(_ entries: [VaultEntry], key: Data) -> Data? {
        var wire: [SealedEntry] = []
        for e in entries {
            guard let sealed = try? vaultSealEntry(vaultKey: key, id: e.id, updatedAtMs: e.updatedAtMs, payload: Data(e.payload.utf8))
            else { continue }
            wire.append(SealedEntry(id: e.id, sealed: sealed))
        }
        guard let data = try? JSONEncoder().encode(wire), data.count <= maxBlobBytes else { return nil }
        return data
    }

    /// An entry that does not open (another key, a changed byte) is skipped and counted.
    static func decodeSealed(_ data: Data, key: Data) -> (entries: [VaultEntry], rejected: Int) {
        guard data.count <= maxBlobBytes, let wire = try? JSONDecoder().decode([SealedEntry].self, from: data) else {
            return ([], 0)
        }
        var out: [VaultEntry] = []
        var rejected = 0
        for w in wire.prefix(1000) {
            guard let opened = try? vaultOpenEntry(vaultKey: key, id: w.id, sealed: w.sealed),
                  let payload = String(data: opened.payload, encoding: .utf8) else { rejected += 1; continue }
            out.append(VaultEntry(id: w.id, updatedAtMs: opened.updatedAtMs, deleted: false, payload: payload))
        }
        return (out, rejected)
    }
}

/// What this device offers for tier B, and how an incoming item is checked and applied.
enum VaultSecretItems {
    static func isEnvName(_ s: String) -> Bool {
        guard let first = s.unicodeScalars.first, s.count <= 64 else { return false }
        let head = CharacterSet.letters.union(CharacterSet(charactersIn: "_"))
        let rest = head.union(.decimalDigits)
        return head.contains(first) && first.isASCII && s.unicodeScalars.allSatisfy { $0.isASCII && rest.contains($0) }
    }

    /// `secret` is the vault key; without it the items carry no payload (enough for a list).
    static func keyItems(default hasDefault: Bool, extra: [SSHKeyInfo], secret: Data? = nil, keychain: KeychainManager = .shared) -> [SecretItem] {
        var out: [SecretItem] = []
        func item(id: String?, name: String) {
            guard let pub = keychain.getOrDerivePublicKey(id: id) else { return }
            guard let secret else {
                out.append(SecretItem(id: SecretIDs.key(id), label: name, payload: ""))
                return
            }
            guard let sealed = try? keySealStable(key: keychain.keyRef(id), secret: secret),
                  let data = try? JSONEncoder().encode(SyncedKey(name: name, sealed: sealed.base64EncodedString(), publicKey: pub)),
                  let json = String(data: data, encoding: .utf8) else { return }
            out.append(SecretItem(id: SecretIDs.key(id), label: name, payload: json))
        }
        if hasDefault { item(id: nil, name: "Default key") }
        for k in extra { item(id: k.id, name: k.name) }
        return out
    }

    static func envItems(_ profiles: [SavedAgentProfile]) -> [SecretItem] {
        var out: [SecretItem] = []
        for p in profiles {
            for line in p.env {
                guard let eq = line.firstIndex(of: "=") else { continue }
                let name = String(line[..<eq])
                let value = String(line[line.index(after: eq)...])
                guard SyncedProfile.isSecretKey(name), isEnvName(name), !value.isEmpty,
                      let data = try? JSONEncoder().encode(SyncedEnv(profile: p.id, name: name, value: value)),
                      let json = String(data: data, encoding: .utf8) else { continue }
                out.append(SecretItem(id: SecretIDs.env(profile: p.id, name: name), label: "\(p.name): \(name)", payload: json))
            }
        }
        return out
    }

    /// Whether an incoming payload may be applied here. `knownProfiles` are the launch profiles this
    /// device has (a value for a profile it does not have waits in iCloud).
    static func accepts(_ item: SecretItem, knownProfiles: Set<String>, hasDefaultKey: Bool, keychain: KeychainManager = .shared) -> Bool {
        let data = Data(item.payload.utf8)
        if item.id.hasPrefix(SecretIDs.keyPrefix) {
            guard let k = try? JSONDecoder().decode(SyncedKey.self, from: data),
                  !k.name.isEmpty, k.name.count <= 100,
                  !k.name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
                  k.sealed.count <= 4096, Data(base64Encoded: k.sealed) != nil,
                  k.publicKey.hasPrefix("ssh-ed25519 "), k.publicKey.count <= 200
            else { return false }
            let keyId = SecretIDs.keyId(from: item.id)
            // The default key is never replaced; the same key arriving again is a no-op.
            if keyId == nil, hasDefaultKey, keychain.getOrDerivePublicKey(id: nil) != k.publicKey { return false }
            return keyId == nil || UUID(uuidString: keyId ?? "") != nil
        }
        if item.id.hasPrefix(SecretIDs.envPrefix) {
            guard let e = try? JSONDecoder().decode(SyncedEnv.self, from: data),
                  knownProfiles.contains(e.profile),
                  isEnvName(e.name), SyncedProfile.isSecretKey(e.name),
                  !e.value.isEmpty, e.value.count <= 4096,
                  !e.value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) || $0 == "\n" }),
                  SecretIDs.env(profile: e.profile, name: e.name) == item.id
            else { return false }
            return true
        }
        return false
    }

    /// Writes an accepted item. A key is stored like an imported one: this device only, never
    /// synchronizable. An env value replaces the line of the same name in the profile.
    @MainActor
    static func apply(_ item: SecretItem, secret: Data, agents: AgentProfilesManager, keys: SSHKeyStore, keychain: KeychainManager = .shared) {
        let data = Data(item.payload.utf8)
        if item.id.hasPrefix(SecretIDs.keyPrefix), let k = try? JSONDecoder().decode(SyncedKey.self, from: data) {
            let keyId = SecretIDs.keyId(from: item.id)
            let existed = keychain.hasPrivateKey(id: keyId)
            guard let sealed = Data(base64Encoded: k.sealed),
                  let info = try? keyUnsealStable(key: keychain.keyRef(keyId), sealed: sealed, secret: secret, overwrite: keyId != nil)
            else { return }
            if info.publicKeyOpenssh.split(separator: " ").prefix(2) != k.publicKey.split(separator: " ").prefix(2) {
                if !existed { try? keychain.deleteKey(id: keyId) }
                return
            }
            keys.adopt(id: keyId, name: k.name)
        } else if let e = try? JSONDecoder().decode(SyncedEnv.self, from: data), var list = Optional(agents.profiles), let i = list.firstIndex(where: { $0.id == e.profile }) {
            list[i].env.removeAll { $0.hasPrefix(e.name + "=") }
            list[i].env.append("\(e.name)=\(e.value)")
            agents.applySynced(list)
        }
    }
}
