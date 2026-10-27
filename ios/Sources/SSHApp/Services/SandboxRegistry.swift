import Foundation
import SshCoreBridge

/// The sandboxes this phone started and has not removed (spec 62). Each one is followed on its
/// own: the Sandbox screen lists them, the push extension is told which banner belongs to which.
/// Kept on the phone; the server holds the containers.
struct SandboxRecord: Codable, Identifiable, Equatable, Sendable {
    var id: String
    /// "claude", "agy", "codex" or "grok" (the same names the allow-lists use).
    var agentName: String
    var folder: String
    var engineName: String
    var online: Bool
    var autoApprove: Bool
    var copyLogin: Bool
    var claudeConfigDir: String
    /// `ServerProfile.id` of the server it runs on; `nil` for the default server.
    var serverId: String?
    /// "Claude Code", "Codex"…, for the list.
    var label: String
    var created: Date

    var agent: SandboxAgent {
        switch agentName {
        case "agy": return .antigravity
        case "codex": return .codex
        case "grok": return .grok
        default: return .claudeCode
        }
    }

    var engine: SandboxEngine { engineName == "podman" ? .podman : .docker }

    func profile(allowHosts: [String]) -> SandboxProfile {
        SandboxProfile(
            engine: engine, agent: agent, sandboxId: id, projectFolder: folder,
            network: online ? .online : .offline, allowHosts: allowHosts,
            cpus: 2, memoryMb: 4096, pids: 512, autoApprove: autoApprove,
            baseImage: sandboxDefaultBaseImage(),
            copyLogin: copyLogin, claudeConfigDir: claudeConfigDir)
    }
}

enum SandboxRegistry {
    static let key = "sandbox.registry"
    static let maxEntries = 24

    static func all(_ defaults: UserDefaults = .standard) -> [SandboxRecord] {
        guard let data = defaults.data(forKey: key),
              let list = try? JSONDecoder().decode([SandboxRecord].self, from: data) else { return [] }
        return list
    }

    private static func save(_ list: [SandboxRecord], _ defaults: UserDefaults) {
        if let data = try? JSONEncoder().encode(list) { defaults.set(data, forKey: key) }
    }

    /// Adds or replaces a record; the oldest fall off past `maxEntries`.
    static func upsert(_ r: SandboxRecord, _ defaults: UserDefaults = .standard) {
        var list = all(defaults).filter { $0.id != r.id }
        list.append(r)
        list.sort { $0.created < $1.created }
        save(Array(list.suffix(maxEntries)), defaults)
    }

    static func remove(_ id: String, _ defaults: UserDefaults = .standard) {
        save(all(defaults).filter { $0.id != id }, defaults)
    }

    /// The sandboxes of one server, newest first. `nil` is the default server and matches only
    /// records made there: a sandbox must never be shown, or acted on, through another server.
    static func records(on server: UUID?, in list: [SandboxRecord]) -> [SandboxRecord] {
        list.filter { $0.serverId == server?.uuidString }
            .sorted { $0.created > $1.created }
    }
}
