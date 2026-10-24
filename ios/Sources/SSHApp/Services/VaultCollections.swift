import Foundation
import SshCoreBridge

/// Launch profiles and snippets travel in the same synced item as the servers (feature 43 §6),
/// each under its own id prefix. These two have no edit time of their own, so the engine notices
/// a local edit by comparing the payload with what it saw last time, and a deletion by an id that
/// was there and is gone. The winner of every id is still decided by the core (`mergeVault`).
struct CollectionItem: Equatable {
    var id: String
    var payload: String
}

struct CollectionState: Codable, Equatable {
    /// Edit time of every id as last seen here, and the payload it had then.
    var stamps: [String: UInt64] = [:]
    var payloads: [String: String] = [:]
    /// The ids that existed locally after the last sync.
    var known: [String] = []
    var tombstones: [String: UInt64] = [:]
}

enum VaultCollectionEngine {
    struct Outcome: Equatable {
        /// Items to create or replace locally (id, payload).
        var upserts: [CollectionItem]
        var deletes: [String]
        /// Everything this device now knows, prefixed, to write back.
        var entries: [VaultEntry]
        var state: CollectionState
    }

    /// `accept` is the caller's check of a payload that came from the other side; a payload it
    /// refuses is not applied (and stays in the vault for the devices that understand it).
    static func reconcile(
        prefix: String, local: [CollectionItem], remote: [VaultEntry], state: CollectionState,
        now: Date, accept: (CollectionItem) -> Bool
    ) -> Outcome {
        let nowMs = VaultSyncEngine.millis(now)
        var st = state
        let live = Set(local.map(\.id))

        // Local edits get a new time; a local deletion becomes a tombstone.
        for item in local where st.payloads[item.id] != item.payload {
            st.payloads[item.id] = item.payload
            st.stamps[item.id] = nowMs
            st.tombstones[item.id] = nil
        }
        for id in st.known where !live.contains(id) {
            st.tombstones[id] = nowMs
            st.stamps[id] = nil
            st.payloads[id] = nil
        }
        for id in live { st.tombstones[id] = nil }

        var own = local.map {
            VaultEntry(id: prefix + $0.id, updatedAtMs: st.stamps[$0.id] ?? nowMs, deleted: false, payload: $0.payload)
        }
        own += st.tombstones.map { VaultEntry(id: prefix + $0.key, updatedAtMs: $0.value, deleted: true, payload: "") }

        let theirs = remote.filter { $0.id.hasPrefix(prefix) }
        let merge = mergeVault(local: own, remote: theirs, nowMs: nowMs)
        let taken = Set(merge.takenFromRemote)

        var upserts: [CollectionItem] = []
        var deletes: [String] = []
        var stillLive = live
        for entry in merge.merged where taken.contains(entry.id) {
            let id = String(entry.id.dropFirst(prefix.count))
            if entry.deleted {
                if live.contains(id) { deletes.append(id); stillLive.remove(id) }
                st.tombstones[id] = entry.updatedAtMs
                st.stamps[id] = nil
                st.payloads[id] = nil
            } else {
                let item = CollectionItem(id: id, payload: entry.payload)
                guard accept(item) else { continue }
                upserts.append(item)
                stillLive.insert(id)
                st.stamps[id] = entry.updatedAtMs
                st.payloads[id] = entry.payload
                st.tombstones[id] = nil
            }
        }
        st.known = stillLive.sorted()
        // Tombstones the core let expire are forgotten here too.
        let kept = Set(merge.merged.filter(\.deleted).map { String($0.id.dropFirst(prefix.count)) })
        st.tombstones = st.tombstones.filter { kept.contains($0.key) }
        return Outcome(upserts: upserts, deletes: deletes, entries: merge.merged, state: st)
    }
}

// MARK: - Launch profiles

enum SyncedProfile {
    static func isSecretKey(_ key: String) -> Bool {
        let k = key.uppercased()
        return ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"].contains { k.contains($0) }
    }

    static let prefix = "p_"

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }

    /// What leaves this device: no value of a variable that looks like a secret, and nothing at
    /// all for a profile with a custom hook command (a command the host would run).
    static func outgoing(_ profiles: [SavedAgentProfile]) -> [CollectionItem] {
        profiles.compactMap { p in
            guard p.hookKindRaw != "custom", p.hookExtras.isEmpty, p.hookOwn.isEmpty else { return nil }
            var q = p
            q.env = p.env.filter { !isSecretKey(String($0.prefix { $0 != "=" })) }
            q.customHookCommand = ""
            q.customHookEvents = []
            q.customHookMatcher = ""
            guard let data = try? encoder().encode(q), let json = String(data: data, encoding: .utf8) else { return nil }
            return CollectionItem(id: p.id, payload: json)
        }
    }

    private static func plain(_ s: String, max: Int, allowEmpty: Bool = true) -> Bool {
        (allowEmpty || !s.isEmpty) && s.count <= max
            && !s.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
    }

    /// The other side is not trusted: everything a launch would use is checked again.
    static func decode(_ item: CollectionItem, allowHooks: Bool = false) -> SavedAgentProfile? {
        guard let p = try? JSONDecoder().decode(SavedAgentProfile.self, from: Data(item.payload.utf8)),
              p.id == item.id, plain(p.id, max: 64, allowEmpty: false),
              plain(p.name, max: 100, allowEmpty: false),
              plain(p.binary, max: 300, allowEmpty: false), !p.binary.hasPrefix("-"),
              p.args.count <= 64, p.args.allSatisfy({ plain($0, max: 1000) }),
              p.env.count <= 64,
              p.env.allSatisfy({ e in
                  let key = String(e.prefix { $0 != "=" })
                  return e.contains("=") && plain(e, max: 2000) && !isSecretKey(key)
                      && key.range(of: "^[A-Za-z_][A-Za-z0-9_]*$", options: .regularExpression) != nil
              }),
              plain(p.workingDir, max: 1000), plain(p.gitBranch, max: 200),
              plain(p.permissionMode, max: 40), plain(p.terminalSchemeId, max: 80),
              plain(p.iconSymbol, max: 80), plain(p.iconColor, max: 40),
              plain(p.localTool, max: 40), plain(p.localModel, max: 200),
              plain(p.approvalMatcher, max: 200), p.approvalEvents.count <= 32,
              allowHooks || (p.hookKindRaw != "custom" && p.hookExtras.isEmpty && p.hookOwn.isEmpty)
        else { return nil }
        return p
    }

    /// The received profile with the secrets this device keeps for the same profile.
    static func merged(_ incoming: SavedAgentProfile, into local: SavedAgentProfile?) -> SavedAgentProfile {
        var p = incoming
        if let local {
            p.env += local.env.filter { isSecretKey(String($0.prefix { $0 != "=" })) }
            // A custom hook the user set up here is not replaced by a synced profile.
            p.customHookCommand = local.customHookCommand
            p.customHookEvents = local.customHookEvents
            p.customHookMatcher = local.customHookMatcher
        }
        return p
    }
}

// MARK: - Snippets

struct SyncedSnippet: Codable, Equatable {
    var id: String
    var title: String
    var command: String
    var category: String
    var tags: [String]

    static let prefix = "s_"
    private static let secret = try? NSRegularExpression(
        pattern: #"(api[_-]?key|secret|token|password|passwd|private[_-]?key)\s*[=:]|authorization:\s*bearer\s+"#,
        options: .caseInsensitive)

    static func looksSecret(_ text: String) -> Bool {
        secret?.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }

    static func outgoing(_ snippets: [SyncedSnippet]) -> [CollectionItem] {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return snippets.compactMap { s in
            guard !looksSecret(s.command), !looksSecret(s.title),
                  let data = try? e.encode(s), let json = String(data: data, encoding: .utf8) else { return nil }
            return CollectionItem(id: s.id, payload: json)
        }
    }

    static func decode(_ item: CollectionItem) -> SyncedSnippet? {
        func plain(_ s: String, max: Int, allowEmpty: Bool = false) -> Bool {
            (allowEmpty || !s.isEmpty) && s.count <= max && !s.unicodeScalars.contains { $0.value == 0 }
        }
        guard let s = try? JSONDecoder().decode(SyncedSnippet.self, from: Data(item.payload.utf8)),
              s.id == item.id, plain(s.id, max: 64), plain(s.title, max: 200), plain(s.command, max: 4000),
              plain(s.category, max: 50, allowEmpty: true), s.tags.count <= 10,
              s.tags.allSatisfy({ plain($0, max: 30) }),
              !looksSecret(s.command)
        else { return nil }
        return s
    }
}
