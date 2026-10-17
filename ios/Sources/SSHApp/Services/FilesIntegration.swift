import Foundation
import FileProvider

/// Keeps the servers visible in the Files app in step with the saved servers: publishes the list
/// for the extension and adds / removes one File Provider domain per server.
@MainActor
public enum FilesIntegration {
    /// Servers that appear in Files: switched on (default) and with a pinned host key.
    static func sharedServers(from profiles: [ServerProfile]) -> [SharedServer] {
        profiles.compactMap { p in
            guard p.showInFiles != false, let key = p.hostKeyFingerprint, !key.isEmpty else { return nil }
            let folder = p.filesFolder?.trimmingCharacters(in: .whitespacesAndNewlines)
            return SharedServer(
                id: p.id.uuidString, name: p.name, host: p.host, port: p.port, username: p.username,
                keyId: p.keyId, hostKey: key, startFolder: (folder?.isEmpty ?? true) ? nil : folder
            )
        }
    }

    public static func refresh(profiles: [ServerProfile]) {
        let servers = sharedServers(from: profiles)
        SharedStore.publish(servers)
        Task { await syncDomains(servers) }
    }

    private static func syncDomains(_ servers: [SharedServer]) async {
        let manager = NSFileProviderManager.self
        let existing: [NSFileProviderDomain] = await withCheckedContinuation { cont in
            manager.getDomainsWithCompletionHandler { domains, _ in cont.resume(returning: domains) }
        }
        let wanted = Dictionary(uniqueKeysWithValues: servers.map { ($0.id, $0.name) })
        for d in existing {
            // A renamed server needs a new domain: the display name cannot be changed.
            if wanted[d.identifier.rawValue] != d.displayName {
                try? await manager.remove(d)
            }
        }
        let kept = Set(existing.filter { wanted[$0.identifier.rawValue] == $0.displayName }.map { $0.identifier.rawValue })
        for s in servers where !kept.contains(s.id) {
            let domain = NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier(rawValue: s.id), displayName: s.name)
            try? await manager.add(domain)
        }
    }
}
