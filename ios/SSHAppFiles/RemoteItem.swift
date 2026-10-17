import Foundation
import FileProvider
import UniformTypeIdentifiers
import SshCoreBridge

/// Item identifiers are `p:` + base64url(absolute remote path); the server root is `.rootContainer`.
enum ItemCodec {
    static func identifier(for path: String, root: String) -> NSFileProviderItemIdentifier {
        path == root ? .rootContainer : NSFileProviderItemIdentifier("p:" + ItemPath.encode(path))
    }

    static func path(for id: NSFileProviderItemIdentifier, root: String) -> String? {
        if id == .rootContainer { return root }
        guard id.rawValue.hasPrefix("p:") else { return nil }
        return ItemPath.decode(String(id.rawValue.dropFirst(2)))
    }

    static func parent(of path: String) -> String { ItemPath.parent(of: path) }
}

final class RemoteItem: NSObject, NSFileProviderItem {
    let entry: RemoteEntry
    let root: String

    init(entry: RemoteEntry, root: String) {
        self.entry = entry
        self.root = root
    }

    static func rootItem(path: String) -> RemoteItem {
        RemoteItem(entry: RemoteEntry(path: path, name: "/", kind: .directory, size: 0, modifiedUnix: 0, mode: 0o755), root: path)
    }

    var itemIdentifier: NSFileProviderItemIdentifier { ItemCodec.identifier(for: entry.path, root: root) }

    var parentItemIdentifier: NSFileProviderItemIdentifier {
        if entry.path == root { return .rootContainer }
        return ItemCodec.identifier(for: ItemCodec.parent(of: entry.path), root: root)
    }

    var filename: String { entry.name }

    var contentType: UTType {
        if entry.kind == .directory { return .folder }
        return UTType(filenameExtension: (entry.name as NSString).pathExtension) ?? .data
    }

    var documentSize: NSNumber? { entry.kind == .directory ? nil : NSNumber(value: entry.size) }

    var contentModificationDate: Date? {
        entry.modifiedUnix > 0 ? Date(timeIntervalSince1970: TimeInterval(entry.modifiedUnix)) : nil
    }

    var capabilities: NSFileProviderItemCapabilities {
        if entry.kind == .directory {
            return [.allowsReading, .allowsContentEnumerating, .allowsAddingSubItems, .allowsRenaming, .allowsReparenting, .allowsDeleting]
        }
        return [.allowsReading, .allowsWriting, .allowsRenaming, .allowsReparenting, .allowsDeleting]
    }

    var itemVersion: NSFileProviderItemVersion {
        NSFileProviderItemVersion(
            contentVersion: Data("\(entry.modifiedUnix):\(entry.size)".utf8),
            metadataVersion: Data("\(entry.modifiedUnix):\(entry.size):\(entry.mode):\(entry.name)".utf8)
        )
    }
}
