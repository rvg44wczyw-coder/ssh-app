import Foundation
import Network
import Combine

public final class NetworkMonitor: ObservableObject {
    public static let shared = NetworkMonitor()

    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "com.sshapp.networkmonitor")

    @Published public private(set) var isConnected: Bool = true
    @Published public private(set) var isExpensive: Bool = false
    @Published public private(set) var isConstrained: Bool = false

    private init() {
        monitor.pathUpdateHandler = { [weak self] path in
            DispatchQueue.main.async {
                self?.isConnected = (path.status == .satisfied)
                self?.isExpensive = path.isExpensive
                self?.isConstrained = path.isConstrained
            }
        }
        monitor.start(queue: queue)
    }

    deinit {
        monitor.cancel()
    }
}

/// Fast, asynchronous TCP reachability probe to determine whether an SSH host
/// is online and listening on its configured port ("pinging").
public enum ServerPingService {

    /// Tests reachability to an SSH server host and port via TCP probe.
    /// Returns true if the TCP connection reaches the .ready state within the timeout.
    public static func ping(host: String, port: UInt16, timeout: TimeInterval = 1.5) async -> Bool {
        let trimmedHost = host.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedHost.isEmpty else { return false }

        return await withCheckedContinuation { continuation in
            let endpoint = NWEndpoint.hostPort(
                host: NWEndpoint.Host(trimmedHost),
                port: NWEndpoint.Port(rawValue: port) ?? 22
            )
            let params = NWParameters.tcp
            params.prohibitExpensivePaths = false
            let connection = NWConnection(to: endpoint, using: params)
            let queue = DispatchQueue(label: "com.sshapp.ping.\(trimmedHost)")

            let lock = NSLock()
            var didResume = false

            let finish = { (result: Bool) in
                lock.lock()
                defer { lock.unlock() }
                if !didResume {
                    didResume = true
                    connection.stateUpdateHandler = nil
                    connection.cancel()
                    continuation.resume(returning: result)
                }
            }

            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    finish(true)
                case .failed, .cancelled:
                    finish(false)
                default:
                    break
                }
            }

            connection.start(queue: queue)

            queue.asyncAfter(deadline: .now() + timeout) {
                finish(false)
            }
        }
    }

    /// Checks reachability of multiple server profiles concurrently.
    /// Returns the array of servers that responded to ping (in original order).
    public static func checkReachableServers(
        profiles: [ServerProfile],
        timeout: TimeInterval = 1.5
    ) async -> [ServerProfile] {
        guard !profiles.isEmpty else { return [] }

        return await withTaskGroup(of: (ServerProfile, Bool).self) { group in
            for profile in profiles {
                group.addTask {
                    let reachable = await ping(host: profile.host, port: profile.port, timeout: timeout)
                    return (profile, reachable)
                }
            }

            var pinging: [ServerProfile] = []
            for await (profile, isReachable) in group {
                if isReachable {
                    pinging.append(profile)
                }
            }
            // Preserve the original order from the profiles list
            return profiles.filter { p in pinging.contains(where: { $0.id == p.id }) }
        }
    }
}
