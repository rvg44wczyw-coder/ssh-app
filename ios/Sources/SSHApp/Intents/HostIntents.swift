import AppIntents
import Foundation
import SshCoreBridge

// Apple Shortcuts and Siri (feature 48). Intents work on saved items only: a server, a saved agent
// profile or a saved command is chosen from a list, and the one free text field (the instruction
// for an agent) is data for the agent, never a command. Nothing here approves a request.

// MARK: Stored items, read the way the app saved them

enum IntentStore {
    static func servers() -> [ServerProfile] {
        decode([ServerProfile].self, key: "saved_server_profiles") ?? []
    }

    static func agentProfiles() -> [SavedAgentProfile] {
        decode([SavedAgentProfile].self, key: "saved_agent_profiles") ?? []
    }

    struct Snippet: Codable { let id: String; let title: String; let command: String }

    /// The user's own saved commands (the built-in suggestions are not offered).
    static func snippets() -> [Snippet] {
        decode([Snippet].self, key: "ssh_app.custom_snippets.v1") ?? []
    }

    private static func decode<T: Decodable>(_ type: T.Type, key: String) -> T? {
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }
}

enum IntentFailure: Error, CustomLocalizedStringResourceConvertible {
    case unknownItem(String)
    case notStarted(String)

    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .unknownItem(let what): return "That \(what) is no longer saved in the app."
        case .notStarted(let why): return "Not started: \(why)"
        }
    }
}

/// Everything an intent needs to reach a host, or the reason it cannot.
struct IntentTarget {
    let server: ServerProfile
    let privateKey: String

    @MainActor
    static func resolve(serverId: String) throws -> IntentTarget {
        guard let server = IntentStore.servers().first(where: { $0.id.uuidString == serverId }) else {
            throw IntentFailure.unknownItem("server")
        }
        guard server.hostKeyFingerprint != nil else {
            throw IntentFailure.notStarted("open the app and connect to this server once, to pin its host key")
        }
        guard let key = KeychainManager.shared.getPrivateKey(id: server.keyId), !key.isEmpty else {
            throw IntentFailure.notStarted("the SSH key is not available (unlock the phone and try again)")
        }
        return IntentTarget(server: server, privateKey: key)
    }

    var config: RemoteServerConfig {
        RemoteServerConfig(host: server.host, port: server.port, username: server.username,
                           privateKeyPem: privateKey, pinnedHostKey: server.hostKeyFingerprint)
    }
}

/// A background intent has about 30 s: past the budget it reports "not started" and stops; it
/// never retries in the background and never reports a success it did not see.
func withIntentBudget<T: Sendable>(seconds: Double = 25, _ work: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await work() }
        group.addTask {
            try await Task.sleep(for: .seconds(seconds))
            throw IntentFailure.notStarted("the host did not answer in time")
        }
        defer { group.cancelAll() }
        guard let first = try await group.next() else { throw IntentFailure.notStarted("no answer") }
        return first
    }
}

// MARK: Entities

struct ServerEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Server"
    static var defaultQuery = ServerEntityQuery()
    var id: String
    var name: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct ServerEntityQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [ServerEntity] {
        IntentStore.servers().filter { identifiers.contains($0.id.uuidString) }
            .map { ServerEntity(id: $0.id.uuidString, name: $0.name) }
    }
    func suggestedEntities() async throws -> [ServerEntity] {
        IntentStore.servers().map { ServerEntity(id: $0.id.uuidString, name: $0.name) }
    }
}

struct AgentProfileEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Agent profile"
    static var defaultQuery = AgentProfileEntityQuery()
    var id: String
    var name: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct AgentProfileEntityQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [AgentProfileEntity] {
        IntentStore.agentProfiles().filter { identifiers.contains($0.id) }
            .map { AgentProfileEntity(id: $0.id, name: $0.name) }
    }
    func suggestedEntities() async throws -> [AgentProfileEntity] {
        IntentStore.agentProfiles().map { AgentProfileEntity(id: $0.id, name: $0.name) }
    }
}

struct SavedCommandEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Saved command"
    static var defaultQuery = SavedCommandEntityQuery()
    var id: String
    var title: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(title)") }
}

struct SavedCommandEntityQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [SavedCommandEntity] {
        IntentStore.snippets().filter { identifiers.contains($0.id) }
            .map { SavedCommandEntity(id: $0.id, title: $0.title) }
    }
    func suggestedEntities() async throws -> [SavedCommandEntity] {
        IntentStore.snippets().map { SavedCommandEntity(id: $0.id, title: $0.title) }
    }
}

// MARK: Intents

/// Read-only facts about a host, for a spoken answer or a chained shortcut.
struct FetchHostHealthIntent: AppIntent {
    static var title: LocalizedStringResource = "Host health"
    static var description = IntentDescription("Load, memory and disk of one of your servers.")

    @Parameter(title: "Server") var server: ServerEntity

    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let target = try await MainActor.run { try IntentTarget.resolve(serverId: server.id) }
        let pin = HostTargetCache.pin(host: target.server.host, port: target.server.port, username: target.server.username)
        let snap = try await withIntentBudget { try await fetchHostTelemetryOnce(config: target.config, helperSha256: pin) }
        let text = String(format: "%@: load %.2f, memory %.0f%%, disk %.0f%%.",
                          target.server.name, snap.load1m, snap.memoryUsagePercent, snap.diskUsagePercent)
        return .result(value: text, dialog: IntentDialog(stringLiteral: text))
    }
}

/// Runs one of the user's saved commands on a host. The command comes from the store, by id.
struct RunSavedCommandIntent: AppIntent {
    static var title: LocalizedStringResource = "Run saved command"
    static var description = IntentDescription("Runs a command you saved in the app on one of your servers.")
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication

    @Parameter(title: "Server") var server: ServerEntity
    @Parameter(title: "Saved command") var command: SavedCommandEntity

    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        guard let saved = IntentStore.snippets().first(where: { $0.id == command.id }) else {
            throw IntentFailure.unknownItem("command")
        }
        if !extractSnippetPlaceholders(command: saved.command).isEmpty {
            throw IntentFailure.notStarted("this command has {{placeholders}}; run it from the app")
        }
        let target = try await MainActor.run { try IntentTarget.resolve(serverId: server.id) }
        let text = saved.command
        let out = try await withIntentBudget { try await executeRemoteCommand(config: target.config, command: text) }
        let shown = String(out.trimmingCharacters(in: .whitespacesAndNewlines).prefix(400))
        let reply = shown.isEmpty ? "\(saved.title) finished on \(target.server.name)." : shown
        return .result(value: reply, dialog: IntentDialog(stringLiteral: reply))
    }
}

/// Starts a saved Claude Code profile in a new background session with an instruction. The
/// instruction is data for the agent. It does not open an approval: approving stays in the app.
struct RunAgentTaskIntent: AppIntent {
    static var title: LocalizedStringResource = "Run agent task"
    static var description = IntentDescription("Starts a saved agent profile on a server with an instruction.")
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication

    @Parameter(title: "Server") var server: ServerEntity
    @Parameter(title: "Agent profile") var profile: AgentProfileEntity
    @Parameter(title: "Instruction") var instruction: String

    func perform() async throws -> some IntentResult & ProvidesDialog {
        guard let saved = IntentStore.agentProfiles().first(where: { $0.id == profile.id }) else {
            throw IntentFailure.unknownItem("agent profile")
        }
        let target = try await MainActor.run { try IntentTarget.resolve(serverId: server.id) }
        let name = "intent-" + String(UUID().uuidString.prefix(8)).lowercased()
        let command: String
        do {
            command = try buildAgentTaskCommand(profile: saved.coreProfile, sessionName: name, prompt: instruction)
        } catch {
            throw IntentFailure.notStarted(error.localizedDescription)
        }
        let out = try await withIntentBudget { try await executeRemoteCommand(config: target.config, command: command) }
        guard out.contains("STARTED") else {
            throw IntentFailure.notStarted(out.contains("NO_TMUX") ? "tmux is not installed on the host" : "the session could not be created")
        }
        return .result(dialog: "\(saved.name) started on \(target.server.name).")
    }
}

/// Which container a spoken or typed name means: an exact name, and only one. Nothing is guessed,
/// because the next step restarts it.
enum DockerIntentMatch {
    static func container(named name: String, in containers: [DockerContainerInfo]) -> DockerContainerInfo? {
        let wanted = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !wanted.isEmpty else { return nil }
        let hits = containers.filter { $0.name == wanted }
        return hits.count == 1 ? hits[0] : nil
    }
}

/// Restarts one container by its exact name, after the user confirms. It changes a running
/// service, so the phone must be unlocked on this device (not just reachable) and the
/// confirmation names the container and the server.
struct RestartDockerContainerIntent: AppIntent {
    static var title: LocalizedStringResource = "Restart Docker container"
    static var description = IntentDescription("Restarts a container on one of your servers, after you confirm.")
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication

    @Parameter(title: "Server") var server: ServerEntity
    @Parameter(title: "Container name") var containerName: String

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let target = try await MainActor.run { try IntentTarget.resolve(serverId: server.id) }
        let listing = try await withIntentBudget {
            parseDockerList(output: try await executeRemoteCommand(config: target.config, command: dockerListCommand()))
        }
        guard listing.dockerAvailable else {
            throw IntentFailure.notStarted(listing.errorMessage ?? "Docker is not available on the host")
        }
        guard let found = DockerIntentMatch.container(named: containerName, in: listing.containers) else {
            throw IntentFailure.notStarted("there is no single container named \"\(containerName)\" on \(target.server.name)")
        }
        try await requestConfirmation(actionName: .continue, dialog: "Restart \(found.name) on \(target.server.name)?")
        let command: String
        do { command = try dockerActionCommand(action: .restart, containerId: found.id) }
        catch { throw IntentFailure.notStarted(error.localizedDescription) }
        let out = try await withIntentBudget { try await executeRemoteCommand(config: target.config, command: command) }
        do { try parseDockerAction(output: out) }
        catch { throw IntentFailure.notStarted(error.localizedDescription) }
        return .result(dialog: "\(found.name) restarted on \(target.server.name).")
    }
}

struct SSHAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: FetchHostHealthIntent(),
            phrases: ["Check \(\.$server) health in \(.applicationName)", "How is \(\.$server) in \(.applicationName)"],
            shortTitle: "Host health",
            systemImageName: "waveform.path.ecg"
        )
        AppShortcut(
            intent: RunSavedCommandIntent(),
            phrases: ["Run a saved command on \(\.$server) in \(.applicationName)"],
            shortTitle: "Run saved command",
            systemImageName: "terminal"
        )
        AppShortcut(
            intent: RunAgentTaskIntent(),
            phrases: ["Start an agent on \(\.$server) in \(.applicationName)"],
            shortTitle: "Run agent task",
            systemImageName: "sparkles"
        )
        AppShortcut(
            intent: RestartDockerContainerIntent(),
            phrases: ["Restart a Docker container in \(.applicationName)"],
            shortTitle: "Restart Docker container",
            systemImageName: "shippingbox"
        )
    }
}
