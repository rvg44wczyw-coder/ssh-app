import Foundation
import FileProvider
import UniformTypeIdentifiers
import SshCoreBridge

/// Files app integration: one domain per server. See docs/features/21-files-provider.md.
final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension {
    private static let maxDownload: UInt64 = 2 * 1024 * 1024 * 1024

    private let domain: NSFileProviderDomain
    private let keyGroup: String?
    private var current: (server: SharedServer, pool: ConnectionPool)?
    private let poolLock = NSLock()
    private let tempDir: URL

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        self.keyGroup = Bundle.main.object(forInfoDictionaryKey: "SSHAppKeychainGroup") as? String
        let manager = NSFileProviderManager(for: domain)
        self.tempDir = (try? manager?.temporaryDirectoryURL()) ?? FileManager.default.temporaryDirectory
        super.init()
    }

    func invalidate() {
        poolLock.lock(); let old = current?.pool; current = nil; poolLock.unlock()
        if let old { Task { await old.close() } }
    }

    /// The pool for this server as the app published it just now: a re-pinned host key, another SSH
    /// key or another root folder takes effect on the next request, not after a restart.
    private func requirePool() throws -> ConnectionPool {
        guard let server = SharedStore.load(group: keyGroup).first(where: { $0.id == domain.identifier.rawValue }) else {
            throw NSFileProviderError(.notAuthenticated)
        }
        poolLock.lock(); defer { poolLock.unlock() }
        if let c = current, c.server == server { return c.pool }
        if let old = current?.pool { Task { await old.close() } }
        let pool = ConnectionPool(server: server, keyGroup: keyGroup)
        current = (server, pool)
        return pool
    }

    /// Runs `work` as a task whose cancellation follows the returned `Progress` (the cancel button in Files).
    private func run(total: Int64 = 1, _ work: @escaping (Progress) async -> Void) -> Progress {
        let progress = Progress(totalUnitCount: total)
        let task = Task { await work(progress) }
        progress.cancellationHandler = { task.cancel() }
        return progress
    }

    // MARK: Items

    func item(for identifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest,
              completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void) -> Progress {
        run { _ in
            do {
                let item = try await self.requirePool().withClient { client, root -> RemoteItem in
                    guard let path = ItemCodec.path(for: identifier, root: root) else { throw NSFileProviderError(.noSuchItem) }
                    if path == root { return RemoteItem.rootItem(path: root) }
                    return RemoteItem(entry: try await client.statRemote(path: path), root: root)
                }
                completionHandler(item, nil)
            } catch {
                completionHandler(nil, fileProviderError(error))
            }
        }
    }

    func fetchContents(for itemIdentifier: NSFileProviderItemIdentifier, version requestedVersion: NSFileProviderItemVersion?,
                       request: NSFileProviderRequest,
                       completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void) -> Progress {
        run { progress in
            do {
                let dest = self.tempDir.appendingPathComponent(UUID().uuidString)
                let item = try await self.requirePool().withClient { client, root -> RemoteItem in
                    guard let path = ItemCodec.path(for: itemIdentifier, root: root) else { throw NSFileProviderError(.noSuchItem) }
                    let entry = try await client.statRemote(path: path)
                    _ = try await client.downloadRemoteFile(
                        path: path, destLocalPath: dest.path, maxBytes: Self.maxDownload,
                        progress: ProgressBridge(progress, total: entry.size)
                    )
                    return RemoteItem(entry: entry, root: root)
                }
                completionHandler(dest, item, nil)
            } catch {
                completionHandler(nil, nil, fileProviderError(error))
            }
        }
    }

    func createItem(basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
                    options: NSFileProviderCreateItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        run { progress in
            do {
                let item = try await self.requirePool().withClient { client, root -> RemoteItem in
                    guard let parent = ItemCodec.path(for: itemTemplate.parentItemIdentifier, root: root) else {
                        throw NSFileProviderError(.noSuchItem)
                    }
                    guard let path = ItemPath.child(parent, itemTemplate.filename) else { throw NSFileProviderError(.noSuchItem) }
                    if itemTemplate.contentType == .folder {
                        try await client.makeRemoteDir(path: path)
                    } else {
                        // A file without contents (empty, or a type the system does not hand over) is created empty.
                        var source = url
                        var emptyFile: URL?
                        if source == nil {
                            let e = self.tempDir.appendingPathComponent(UUID().uuidString)
                            FileManager.default.createFile(atPath: e.path, contents: Data())
                            emptyFile = e
                            source = e
                        }
                        defer { if let emptyFile { try? FileManager.default.removeItem(at: emptyFile) } }
                        guard let source else { throw NSFileProviderError(.noSuchItem) }
                        let size = (try? FileManager.default.attributesOfItem(atPath: source.path)[.size] as? UInt64) ?? 0
                        _ = try await client.uploadRemoteFile(
                            srcLocalPath: source.path, destPath: path,
                            overwrite: options.contains(.mayAlreadyExist),
                            progress: ProgressBridge(progress, total: size)
                        )
                    }
                    return RemoteItem(entry: try await client.statRemote(path: path), root: root)
                }
                completionHandler(item, [], false, nil)
            } catch {
                completionHandler(nil, [], false, fileProviderError(error))
            }
        }
    }

    func modifyItem(_ item: NSFileProviderItem, baseVersion version: NSFileProviderItemVersion,
                    changedFields: NSFileProviderItemFields, contents newContents: URL?,
                    options: NSFileProviderModifyItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        run { progress in
            do {
                let result = try await self.requirePool().withClient { client, root -> RemoteItem in
                    guard var path = ItemCodec.path(for: item.itemIdentifier, root: root), path != root else {
                        throw NSFileProviderError(.noSuchItem)
                    }
                    if changedFields.contains(.contents), let newContents {
                        let size = (try? FileManager.default.attributesOfItem(atPath: newContents.path)[.size] as? UInt64) ?? 0
                        _ = try await client.uploadRemoteFile(
                            srcLocalPath: newContents.path, destPath: path, overwrite: true,
                            progress: ProgressBridge(progress, total: size)
                        )
                    }
                    if changedFields.contains(.filename) || changedFields.contains(.parentItemIdentifier) {
                        guard let parent = ItemCodec.path(for: item.parentItemIdentifier, root: root) else {
                            throw NSFileProviderError(.noSuchItem)
                        }
                        guard let target = ItemPath.child(parent, item.filename) else { throw NSFileProviderError(.noSuchItem) }
                        if target != path {
                            try await client.renameRemote(from: path, to: target)
                            path = target
                        }
                    }
                    return RemoteItem(entry: try await client.statRemote(path: path), root: root)
                }
                completionHandler(result, [], false, nil)
            } catch {
                completionHandler(nil, [], false, fileProviderError(error))
            }
        }
    }

    func deleteItem(identifier: NSFileProviderItemIdentifier, baseVersion version: NSFileProviderItemVersion,
                    options: NSFileProviderDeleteItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (Error?) -> Void) -> Progress {
        run { _ in
            do {
                try await self.requirePool().withClient { client, root in
                    guard let path = ItemCodec.path(for: identifier, root: root), path != root else {
                        throw NSFileProviderError(.noSuchItem)
                    }
                    // Files and empty folders only; a folder with contents is refused, never deleted recursively.
                    try await client.deleteRemote(path: path)
                }
                completionHandler(nil)
            } catch {
                completionHandler(fileProviderError(error))
            }
        }
    }

    func enumerator(for containerItemIdentifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest) throws -> NSFileProviderEnumerator {
        if containerItemIdentifier == .workingSet { return WorkingSetEnumerator() }
        return RemoteEnumerator(pool: try requirePool(), container: containerItemIdentifier)
    }
}
