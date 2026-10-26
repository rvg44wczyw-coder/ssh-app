import Foundation
import SshCoreBridge

/// The groups of a backup file (feature 56 section 5), built from and applied to the app's data.
/// The format, the encryption and the limits are Rust (`backup_export`, `backup_open`); this file
/// decides what each group holds and checks every record of an imported file with the same
/// validators as an incoming iCloud entry. A group's name decides whether it is secret (see
/// `SECRET_GROUP_IDS` in the core): keys, secret values and hook commands force encryption.
enum BackupService {
    enum GroupID {
        static let servers = "servers"
        static let profiles = "profiles"
        static let snippets = "snippets"
        static let settings = "settings"
        static let keys = "private-keys"
        static let secretValues = "secret-values"
        static let hooks = "hook-commands"
        static let sandboxHosts = "sandbox-hosts"
        static let all = [servers, profiles, snippets, settings, sandboxHosts, keys, secretValues, hooks]
    }

    static func title(_ id: String) -> String {
        switch id {
        case GroupID.servers: return "Servers"
        case GroupID.profiles: return "Launch profiles"
        case GroupID.snippets: return "Snippets"
        case GroupID.settings: return "App settings"
        case GroupID.sandboxHosts: return "Sandbox allowed hosts"
        case GroupID.keys: return "Private keys"
        case GroupID.secretValues: return "Secret environment values"
        case GroupID.hooks: return "Profiles with hook commands"
        default: return id
        }
    }

    /// What the user ticked.
    struct Selection: Equatable {
        var groups: Set<String> = [GroupID.servers, GroupID.profiles, GroupID.snippets, GroupID.settings, GroupID.sandboxHosts]
        /// Pinned host fingerprints travel with the servers only with this.
        var fingerprints = false
        /// Private keys are ticked one by one; none by default.
        var keyIDs: Set<String> = []
    }

    struct KeyRecord: Codable, Equatable {
        var id: String   // "default" for the first key
        var name: String
        /// The private key sealed by the core under the backup passphrase (base64); the key text is
        /// never in the file or in Swift. Empty in the list of choices.
        var sealed: String
        var publicKey: String
    }

    struct ServerRecord: Codable, Equatable {
        var id: String
        var name: String
        var host: String
        var port: UInt16
        var username: String
        var isDefault: Bool
        var fingerprint: String?
    }

    struct Item: Codable, Equatable {
        var id: String
        var payload: String
    }

    struct SecretValues: Codable, Equatable {
        var profileId: String
        var env: [String]
    }

    private static func json<T: Encodable>(_ value: T) -> String {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return (try? e.encode(value)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    }

    // MARK: Building

    /// The groups of a file. Secret environment values and hook commands are separate groups, so the
    /// plain groups never carry them.
    static func buildGroups(servers: [ServerProfile], profiles: [SavedAgentProfile],
                            snippets: [VaultSync.StoredSnippet], configJSON: String,
                            keys: [KeyRecord], selection: Selection,
                            sandboxHosts: [CollectionItem] = []) -> [BackupGroup] {
        var out: [BackupGroup] = []
        let chosen = selection.groups
        if chosen.contains(GroupID.servers), !servers.isEmpty {
            let list = servers.map {
                ServerRecord(id: $0.id.uuidString, name: $0.name, host: $0.host, port: $0.port, username: $0.username,
                             isDefault: $0.isDefault, fingerprint: selection.fingerprints ? $0.hostKeyFingerprint : nil)
            }
            out.append(BackupGroup(id: GroupID.servers, secret: false, json: json(list)))
        }
        if chosen.contains(GroupID.profiles) {
            let items = SyncedProfile.outgoing(profiles).map { Item(id: $0.id, payload: $0.payload) }
            if !items.isEmpty { out.append(BackupGroup(id: GroupID.profiles, secret: false, json: json(items))) }
        }
        if chosen.contains(GroupID.snippets) {
            let plain = snippets.map { SyncedSnippet(id: $0.id, title: $0.title, command: $0.command, category: $0.category, tags: $0.tags) }
            let items = SyncedSnippet.outgoing(plain).map { Item(id: $0.id, payload: $0.payload) }
            if !items.isEmpty { out.append(BackupGroup(id: GroupID.snippets, secret: false, json: json(items))) }
        }
        if chosen.contains(GroupID.settings), !configJSON.isEmpty {
            out.append(BackupGroup(id: GroupID.settings, secret: false, json: json(["config": configJSON])))
        }
        if chosen.contains(GroupID.sandboxHosts), !sandboxHosts.isEmpty {
            let items = sandboxHosts.map { Item(id: $0.id, payload: $0.payload) }
            out.append(BackupGroup(id: GroupID.sandboxHosts, secret: false, json: json(items)))
        }
        if chosen.contains(GroupID.keys) {
            let picked = keys.filter { selection.keyIDs.contains($0.id) }
            if !picked.isEmpty { out.append(BackupGroup(id: GroupID.keys, secret: true, json: json(picked))) }
        }
        if chosen.contains(GroupID.secretValues) {
            let values = profiles.compactMap { p -> SecretValues? in
                let secret = p.env.filter { SyncedProfile.isSecretKey(String($0.prefix { $0 != "=" })) }
                return secret.isEmpty ? nil : SecretValues(profileId: p.id, env: secret)
            }
            if !values.isEmpty { out.append(BackupGroup(id: GroupID.secretValues, secret: true, json: json(values))) }
        }
        if chosen.contains(GroupID.hooks) {
            let withHooks = profiles.filter { $0.hookKindRaw == "custom" || !$0.hookExtras.isEmpty || !$0.hookOwn.isEmpty }
            let items = withHooks.compactMap { p -> Item? in
                var q = p
                q.env = p.env.filter { !SyncedProfile.isSecretKey(String($0.prefix { $0 != "=" })) }
                let e = JSONEncoder()
                e.outputFormatting = [.sortedKeys]
                guard let data = try? e.encode(q), let text = String(data: data, encoding: .utf8) else { return nil }
                return Item(id: p.id, payload: text)
            }
            if !items.isEmpty { out.append(BackupGroup(id: GroupID.hooks, secret: true, json: json(items))) }
        }
        return out
    }

    // MARK: Reading (every record is checked again)

    static func servers(in group: BackupGroup) -> [ServerRecord] {
        guard let list = try? JSONDecoder().decode([ServerRecord].self, from: Data(group.json.utf8)), list.count <= 500 else { return [] }
        return list.filter { r in
            UUID(uuidString: r.id) != nil
                && SyncedServer(name: r.name, host: r.host, port: r.port, username: r.username, fingerprint: r.fingerprint).isSane
                && (r.fingerprint.map { $0.hasPrefix("SHA256:") && $0.count <= 100 } ?? true)
        }
    }

    private static func items(in group: BackupGroup) -> [CollectionItem] {
        guard let list = try? JSONDecoder().decode([Item].self, from: Data(group.json.utf8)), list.count <= 500 else { return [] }
        return list.map { CollectionItem(id: $0.id, payload: $0.payload) }
    }

    static func profiles(in group: BackupGroup, allowHooks: Bool = false) -> [SavedAgentProfile] {
        items(in: group).compactMap { SyncedProfile.decode($0, allowHooks: allowHooks) }
    }

    static func snippets(in group: BackupGroup) -> [VaultSync.StoredSnippet] {
        items(in: group).compactMap(SyncedSnippet.decode).map {
            VaultSync.StoredSnippet(id: $0.id, title: $0.title, command: $0.command, category: $0.category, tags: $0.tags, useCount: 0)
        }
    }

    static func sandboxHosts(in group: BackupGroup) -> [SyncedSandboxHost] {
        let all = items(in: group).compactMap(SyncedSandboxHost.decode)
        return Array(all.prefix(SandboxAllowStore.agents.count * SandboxAllowStore.maxPerAgent))
    }

    static func settingsJSON(in group: BackupGroup) -> String? {
        guard let d = try? JSONDecoder().decode([String: String].self, from: Data(group.json.utf8)),
              let config = d["config"], config.utf8.count <= 1_000_000 else { return nil }
        return config
    }

    static func keys(in group: BackupGroup) -> [KeyRecord] {
        guard let list = try? JSONDecoder().decode([KeyRecord].self, from: Data(group.json.utf8)), list.count <= 50 else { return [] }
        return list.filter {
            !$0.id.isEmpty && $0.id.count <= 64 && $0.name.count <= 100
                && $0.sealed.count <= 8192 && Data(base64Encoded: $0.sealed) != nil
                && $0.publicKey.hasPrefix("ssh-ed25519 ") && $0.publicKey.count <= 200
        }
    }

    static func secretValues(in group: BackupGroup) -> [SecretValues] {
        guard let list = try? JSONDecoder().decode([SecretValues].self, from: Data(group.json.utf8)), list.count <= 500 else { return [] }
        return list.filter { v in
            v.env.count <= 64 && v.env.allSatisfy { e in
                let key = String(e.prefix { $0 != "=" })
                return e.contains("=") && e.count <= 2000 && SyncedProfile.isSecretKey(key)
                    && key.range(of: "^[A-Za-z_][A-Za-z0-9_]*$", options: .regularExpression) != nil
                    && !e.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
            }
        }
    }

    // MARK: Preview

    struct PreviewRow: Identifiable, Equatable {
        let id: String
        let title: String
        let count: Int
        /// Names of the records, for the user to recognise them.
        let names: [String]
        /// Commands the host would run (hook group): shown and confirmed before import.
        let commands: [String]
    }

    static func preview(_ contents: BackupContents) -> [PreviewRow] {
        contents.groups.compactMap { g in
            switch g.id {
            case GroupID.servers:
                let l = servers(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map(\.name), commands: [])
            case GroupID.profiles:
                let l = profiles(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map(\.name), commands: [])
            case GroupID.snippets:
                let l = snippets(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map(\.title), commands: [])
            case GroupID.sandboxHosts:
                let l = sandboxHosts(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map { "\($0.agent): \($0.host)" }, commands: [])
            case GroupID.settings:
                return settingsJSON(in: g) == nil ? nil : PreviewRow(id: g.id, title: title(g.id), count: 1, names: [], commands: [])
            case GroupID.keys:
                let l = keys(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map(\.name), commands: [])
            case GroupID.secretValues:
                let l = secretValues(in: g)
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: [], commands: [])
            case GroupID.hooks:
                let l = profiles(in: g, allowHooks: true)
                let commands = l.flatMap { p -> [String] in
                    var c: [String] = []
                    if p.hookKindRaw == "custom", !p.customHookCommand.isEmpty { c.append("\(p.name): \(p.customHookCommand)") }
                    c += p.hookExtras.map { "\(p.name) · \($0.event): \($0.command)" }
                    c += p.hookOwn.map { "\(p.name) · \($0.event): \($0.command)" }
                    return c
                }
                return PreviewRow(id: g.id, title: title(g.id), count: l.count, names: l.map(\.name), commands: commands)
            default:
                return nil   // a group this version does not know is ignored
            }
        }
    }
}
