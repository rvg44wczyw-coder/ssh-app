import Foundation
import FileProvider
import CryptoKit
import SshCoreBridge

/// Lists one remote folder. Change tracking is done by comparing the folder with the last listing
/// kept in memory; if the extension was restarted in between, the anchor has expired and the system
/// simply lists the folder again.
final class RemoteEnumerator: NSObject, NSFileProviderEnumerator {
    private let pool: ConnectionPool
    private let container: NSFileProviderItemIdentifier

    /// folder path → (fingerprint of the listing, item id → version) of the last listing handed out.
    private static let lock = NSLock()
    nonisolated(unsafe) private static var snapshots: [String: (hash: String, versions: [String: String])] = [:]

    init(pool: ConnectionPool, container: NSFileProviderItemIdentifier) {
        self.pool = pool
        self.container = container
    }

    func invalidate() {}

    private func list() async throws -> (path: String, root: String, items: [RemoteItem]) {
        try await pool.withClient { client, root in
            guard let path = ItemCodec.path(for: container, root: root) else { throw NSFileProviderError(.noSuchItem) }
            let entries = try await client.listRemoteEntries(path: path, includeHidden: false)
            return (path, root, entries.map { RemoteItem(entry: $0, root: root) })
        }
    }

    private static func fingerprint(_ items: [RemoteItem]) -> (String, [String: String]) {
        var versions: [String: String] = [:]
        var hasher = SHA256()
        for i in items {
            let v = "\(i.entry.modifiedUnix):\(i.entry.size):\(i.entry.mode):\(i.entry.name)"
            versions[i.itemIdentifier.rawValue] = v
            hasher.update(data: Data((i.itemIdentifier.rawValue + "=" + v + "\n").utf8))
        }
        return (hasher.finalize().map { String(format: "%02x", $0) }.joined(), versions)
    }

    private func remember(path: String, items: [RemoteItem]) -> NSFileProviderSyncAnchor {
        let (hash, versions) = Self.fingerprint(items)
        Self.lock.lock(); Self.snapshots[path] = (hash, versions); Self.lock.unlock()
        return NSFileProviderSyncAnchor(Data("\(path)\n\(hash)".utf8))
    }

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt page: NSFileProviderPage) {
        Task {
            do {
                let r = try await list()
                _ = remember(path: r.path, items: r.items)
                observer.didEnumerate(r.items)
                observer.finishEnumerating(upTo: nil)
            } catch {
                observer.finishEnumeratingWithError(fileProviderError(error))
            }
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        Task {
            if let r = try? await list() {
                completionHandler(remember(path: r.path, items: r.items))
            } else {
                completionHandler(nil)
            }
        }
    }

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        Task {
            do {
                let parts = String(decoding: anchor.rawValue, as: UTF8.self).split(separator: "\n", maxSplits: 1).map(String.init)
                let r = try await list()
                let old = Self.lock.withLock { Self.snapshots[r.path] }
                guard parts.count == 2, parts[0] == r.path, let old, old.hash == parts[1] else {
                    observer.finishEnumeratingWithError(NSFileProviderError(.syncAnchorExpired))
                    return
                }
                let (hash, versions) = Self.fingerprint(r.items)
                if hash != old.hash {
                    let changed = r.items.filter { old.versions[$0.itemIdentifier.rawValue] != versions[$0.itemIdentifier.rawValue] }
                    let gone = old.versions.keys.filter { versions[$0] == nil }.map { NSFileProviderItemIdentifier($0) }
                    if !changed.isEmpty { observer.didUpdate(changed) }
                    if !gone.isEmpty { observer.didDeleteItems(withIdentifiers: gone) }
                }
                let next = remember(path: r.path, items: r.items)
                observer.finishEnumeratingChanges(upTo: next, moreComing: false)
            } catch {
                observer.finishEnumeratingWithError(fileProviderError(error))
            }
        }
    }
}

/// The working set: the system's index of what to track. Remote changes are discovered when a
/// folder is opened or refreshed (nothing polls in the background), so it stays empty.
final class WorkingSetEnumerator: NSObject, NSFileProviderEnumerator {
    private static let anchor = NSFileProviderSyncAnchor(Data("workingset-1".utf8))
    func invalidate() {}
    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt page: NSFileProviderPage) {
        observer.finishEnumerating(upTo: nil)
    }
    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler(Self.anchor)
    }
    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        observer.finishEnumeratingChanges(upTo: Self.anchor, moreComing: false)
    }
}
