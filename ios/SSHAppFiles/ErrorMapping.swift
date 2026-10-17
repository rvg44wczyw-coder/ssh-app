import Foundation
import FileProvider
import SshCoreBridge

/// Core errors as the errors the Files app understands. Policy refusals look like "no such item".
func fileProviderError(_ error: Error) -> Error {
    if error is CancellationError { return NSFileProviderError(.cannotSynchronize) }
    if let e = error as? NSFileProviderError { return e }
    guard let core = error as? SshCoreError else { return NSFileProviderError(.cannotSynchronize) }
    switch core {
    case .FileNotFound, .NotARegularFile, .InvalidInput:
        return NSFileProviderError(.noSuchItem)
    case .AlreadyExists:
        return NSFileProviderError(.filenameCollision)
    case .TooLarge:
        return NSFileProviderError(.insufficientQuota)
    case .AuthenticationError:
        return NSFileProviderError(.notAuthenticated)
    case .Cancelled:
        return NSFileProviderError(.cannotSynchronize)
    case .ConnectionError, .HostKeyChanged, .Timeout, .NotConnected, .ChannelError:
        return NSFileProviderError(.serverUnreachable)
    default:
        return NSFileProviderError(.cannotSynchronize)
    }
}

/// Bridges the system's `Progress` (cancel button in Files) to the core's transfer callbacks.
final class ProgressBridge: TransferProgress, @unchecked Sendable {
    private let progress: Progress

    init(_ progress: Progress, total: UInt64) {
        self.progress = progress
        if total > 0 { progress.totalUnitCount = Int64(min(total, UInt64(Int64.max))) }
    }

    func onProgress(bytes: UInt64) {
        progress.completedUnitCount = Int64(min(bytes, UInt64(Int64.max)))
    }

    func isCancelled() -> Bool { progress.isCancelled }
}
