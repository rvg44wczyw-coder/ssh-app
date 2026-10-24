import Foundation
import SshCoreBridge

/// Export and import of a backup file for the Backup screen (feature 56 section 5). Cryptography,
/// format and limits are the core's; the record checks are in `BackupService`.
@MainActor
final class BackupController: ObservableObject {
    enum ImportMode: String, CaseIterable, Identifiable {
        case skip, merge, replace
        var id: String { rawValue }
        var title: String {
            switch self {
            case .skip: return "Skip"
            case .merge: return "Add new"
            case .replace: return "Replace same"
            }
        }
    }

    @Published var selection = BackupService.Selection()
    @Published private(set) var contents: BackupContents?
    @Published private(set) var preview: [BackupService.PreviewRow] = []
    @Published var modes: [String: ImportMode] = [:]
    @Published var commandsConfirmed = false
    @Published var error: String?
    @Published var report: String?

    private var pendingFile: Data?

    // MARK: Export

    /// Everything a key group needs: the default key and the extra ones, with their names.
    func keyRecords() -> [BackupService.KeyRecord] {
        var out: [BackupService.KeyRecord] = []
        if let pem = KeychainManager.shared.getPrivateKey(id: nil) {
            out.append(.init(id: "default", name: "Default key", privateKey: pem))
        }
        for k in SSHKeyStore.shared.extra {
            if let pem = KeychainManager.shared.getPrivateKey(id: k.id) {
                out.append(.init(id: k.id, name: k.name, privateKey: pem))
            }
        }
        return out
    }

    /// Writes the file into a temporary folder and returns its URL (for the share sheet or Files).
    func export(servers: ServerProfilesManager, agents: AgentProfilesManager, passphrase: Data?) async -> URL? {
        error = nil
        Self.removeExports()
        let secret = !selection.keyIDs.isEmpty && selection.groups.contains(BackupService.GroupID.keys)
            || selection.groups.contains(BackupService.GroupID.secretValues)
            || selection.groups.contains(BackupService.GroupID.hooks)
        if secret, !(await AppLock.shared.confirmOwner(reason: "Confirm to export secrets")) {
            error = "Not confirmed"
            return nil
        }
        let snippets = (UserDefaults.standard.data(forKey: VaultSync.snippetsStorageKey))
            .flatMap { try? JSONDecoder().decode([VaultSync.StoredSnippet].self, from: $0) } ?? []
        let groups = BackupService.buildGroups(
            servers: servers.profiles, profiles: agents.profiles, snippets: snippets,
            configJSON: UserDefaults.standard.string(forKey: AppConfigStore.storageKey) ?? "",
            keys: keyRecords(), selection: selection)
        guard !groups.isEmpty else { error = "Nothing selected to export"; return nil }
        do {
            let data = try backupExport(groups: groups, passphrase: passphrase,
                                        createdAtMs: UInt64(Date().timeIntervalSince1970 * 1000),
                                        appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("backup-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.complete])
            let stamp = ISO8601DateFormatter().string(from: Date()).prefix(10)
            let url = dir.appendingPathComponent("ssh-app-\(stamp).sshbackup")
            try data.write(to: url, options: [.atomic, .completeFileProtection])
            return url
        } catch {
            self.error = error.localizedDescription
            return nil
        }
    }

    // MARK: Import

    /// Reads a file the user picked. `needsPassphrase` is true for an encrypted one that was opened
    /// without it.
    func load(_ data: Data, passphrase: Data?) -> Bool {
        error = nil
        report = nil
        do {
            let c = try backupOpen(file: data, passphrase: passphrase)
            contents = c
            preview = BackupService.preview(c)
            modes = Dictionary(preview.map { ($0.id, ImportMode.merge) }, uniquingKeysWith: { a, _ in a })
            commandsConfirmed = false
            pendingFile = nil
            return true
        } catch SshCoreError.VaultWrongPassphrase {
            error = "Wrong passphrase or damaged file"
        } catch {
            self.error = error.localizedDescription
        }
        return false
    }

    func isEncrypted(_ data: Data) -> Bool { backupIsEncrypted(file: data) }

    /// Removes the exported files from the temporary folder: a plain backup lists the servers.
    static func removeExports() {
        let tmp = FileManager.default.temporaryDirectory
        let names = (try? FileManager.default.contentsOfDirectory(atPath: tmp.path)) ?? []
        for n in names where n.hasPrefix("backup-") { try? FileManager.default.removeItem(at: tmp.appendingPathComponent(n)) }
    }

    func reset() {
        contents = nil
        preview = []
        modes = [:]
        commandsConfirmed = false
        error = nil
    }

    /// Whether the chosen modes can be applied: commands the host would run need an explicit yes.
    var canApply: Bool {
        let hooksOn = (modes[BackupService.GroupID.hooks] ?? .skip) != .skip
        return modes.values.contains { $0 != .skip } && (!hooksOn || commandsConfirmed)
    }

    func apply(servers: ServerProfilesManager, agents: AgentProfilesManager) async {
        guard let contents, canApply else { return }
        var lines: [String] = []
        func mode(_ id: String) -> ImportMode { modes[id] ?? .skip }
        let touchesKeys = mode(BackupService.GroupID.keys) != .skip || mode(BackupService.GroupID.secretValues) != .skip
            || mode(BackupService.GroupID.hooks) != .skip
        if touchesKeys, !(await AppLock.shared.confirmOwner(reason: "Confirm to import secrets")) {
            error = "Not confirmed"
            return
        }
        for g in contents.groups {
            let m = mode(g.id)
            guard m != .skip else { continue }
            let overwrite = m == .replace
            switch g.id {
            case BackupService.GroupID.servers:
                let list = BackupService.servers(in: g).compactMap { r -> ServerProfile? in
                    UUID(uuidString: r.id).map {
                        ServerProfile(id: $0, name: r.name, host: r.host, port: r.port, username: r.username,
                                      isDefault: false, hostKeyFingerprint: r.fingerprint, updatedAt: Date())
                    }
                }
                lines.append("Servers: \(servers.restore(list, overwrite: overwrite)) added or replaced")
            case BackupService.GroupID.profiles:
                lines.append("Profiles: \(agents.restore(BackupService.profiles(in: g), overwrite: overwrite)) added or replaced")
            case BackupService.GroupID.snippets:
                var local = (UserDefaults.standard.data(forKey: VaultSync.snippetsStorageKey))
                    .flatMap { try? JSONDecoder().decode([VaultSync.StoredSnippet].self, from: $0) } ?? []
                var n = 0
                for s in BackupService.snippets(in: g) {
                    if let i = local.firstIndex(where: { $0.id == s.id }) {
                        guard overwrite else { continue }
                        var t = s
                        t.useCount = local[i].useCount
                        local[i] = t
                    } else {
                        local.append(s)
                    }
                    n += 1
                }
                if n > 0, let data = try? JSONEncoder().encode(local) { UserDefaults.standard.set(data, forKey: VaultSync.snippetsStorageKey) }
                lines.append("Snippets: \(n) added or replaced")
            case BackupService.GroupID.settings:
                // One document: it replaces the settings or is not taken.
                guard overwrite, let text = BackupService.settingsJSON(in: g),
                      let normalized = try? saveAppConfig(config: loadAppConfig(storedJson: text)) else {
                    lines.append("Settings: not changed (choose Replace to apply)")
                    continue
                }
                AppConfigStore.shared.replace(loadAppConfig(storedJson: normalized))
                lines.append("Settings: replaced")
            case BackupService.GroupID.keys:
                lines.append("Keys: \(await importKeys(BackupService.keys(in: g))) imported")
            case BackupService.GroupID.secretValues:
                lines.append("Secret values: \(agents.restoreSecretValues(BackupService.secretValues(in: g), overwrite: overwrite)) set")
            case BackupService.GroupID.hooks:
                lines.append("Hook commands: \(agents.restore(BackupService.profiles(in: g, allowHooks: true), overwrite: overwrite, keepLocalHooks: false)) profiles")
            default:
                break
            }
        }
        report = lines.joined(separator: "\n")
        self.contents = nil
        preview = []
        modes = [:]
    }

    /// New keys only: one whose public half is already here is left alone, never overwritten.
    private func importKeys(_ records: [BackupService.KeyRecord]) async -> Int {
        var existing = Set<String>()
        if let p = KeychainManager.shared.getPublicKey(id: nil) { existing.insert(Self.keyBody(p)) }
        for k in SSHKeyStore.shared.extra { if let p = KeychainManager.shared.getPublicKey(id: k.id) { existing.insert(Self.keyBody(p)) } }
        var n = 0
        for r in records {
            guard let imported = try? importPrivateKey(armoredKey: r.privateKey.trimmingCharacters(in: .whitespacesAndNewlines) + "\n", passphrase: nil),
                  !existing.contains(Self.keyBody(imported.publicKeyOpenssh)) else { continue }
            if (try? await SSHKeyStore.shared.importKey(name: r.name, privateKey: r.privateKey)) != nil {
                existing.insert(Self.keyBody(imported.publicKeyOpenssh))
                n += 1
            }
        }
        return n
    }

    /// The type and the key material of a public key line, without the comment.
    private static func keyBody(_ line: String) -> String {
        line.split(separator: " ").prefix(2).joined(separator: " ")
    }
}
