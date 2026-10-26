import SwiftUI
import UniformTypeIdentifiers
import SshCoreBridge

/// Settings → Backup: export and import of the app's data (feature 56 section 5).
struct BackupSettingsPage: View {
    @ObservedObject var serversManager: ServerProfilesManager
    @ObservedObject var agentsManager: AgentProfilesManager
    @StateObject private var backup = BackupController()
    @ObservedObject private var keyStore = SSHKeyStore.shared

    @State private var phrase = ""
    @State private var phraseAgain = ""
    @State private var exportURL: URL?
    @State private var exporting = false
    @State private var picking = false
    @State private var pickedData: Data?
    @State private var askPassphrase = false
    @State private var importPhrase = ""

    private var hasSecrets: Bool {
        let g = backup.selection.groups
        return (g.contains(BackupService.GroupID.keys) && !backup.selection.keyIDs.isEmpty)
            || g.contains(BackupService.GroupID.secretValues) || g.contains(BackupService.GroupID.hooks)
    }

    private var phraseOK: Bool {
        !hasSecrets || (passphrasesMatch(a: Data(phrase.utf8), b: Data(phraseAgain.utf8)) && passphraseStrength(passphrase: Data(phrase.utf8)).acceptable)
    }

    private func toggle(_ id: String) -> Binding<Bool> {
        Binding(get: { backup.selection.groups.contains(id) },
                set: { on in if on { backup.selection.groups.insert(id) } else { backup.selection.groups.remove(id) } })
    }

    private func keyToggle(_ id: String) -> Binding<Bool> {
        Binding(get: { backup.selection.keyIDs.contains(id) },
                set: { on in if on { backup.selection.keyIDs.insert(id) } else { backup.selection.keyIDs.remove(id) } })
    }

    var body: some View {
        Form {
            exportSection
            importSection
            if let r = backup.report {
                Section("Result") { Text(r).font(.footnote) }
            }
            if let e = backup.error {
                Section { Label(e, systemImage: "exclamationmark.triangle").foregroundColor(.red).font(.footnote) }
            }
        }
        .navigationTitle("Backup")
        .onDisappear { BackupController.removeExports() }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.data]) { result in
            guard case .success(let url) = result else { return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 5 * 1024 * 1024,
                  let data = try? Data(contentsOf: url) else {
                backup.error = "This file is too large to be a backup"
                return
            }
            pickedData = data
            if backup.isEncrypted(data) { importPhrase = ""; askPassphrase = true } else { _ = backup.load(data, passphrase: nil) }
        }
        .alert("Passphrase", isPresented: $askPassphrase) {
            SecureField("Backup passphrase", text: $importPhrase)
            Button("Open") {
                if let d = pickedData { _ = backup.load(d, passphrase: Data(importPhrase.utf8)) }
                importPhrase = ""
            }
            Button("Cancel", role: .cancel) { importPhrase = "" }
        } message: {
            Text("This backup is encrypted.")
        }
    }

    private var exportSection: some View {
        Group {
            Section {
                Toggle("Servers", isOn: toggle(BackupService.GroupID.servers))
                if backup.selection.groups.contains(BackupService.GroupID.servers) {
                    Toggle("Include pinned host fingerprints", isOn: $backup.selection.fingerprints)
                }
                Toggle("Launch profiles", isOn: toggle(BackupService.GroupID.profiles))
                Toggle("Snippets", isOn: toggle(BackupService.GroupID.snippets))
                Toggle("App settings", isOn: toggle(BackupService.GroupID.settings))
                Toggle("Sandbox allowed hosts", isOn: toggle(BackupService.GroupID.sandboxHosts))
                Toggle("Secret environment values", isOn: toggle(BackupService.GroupID.secretValues))
                Toggle("Profiles with hook commands", isOn: toggle(BackupService.GroupID.hooks))
            } header: { Text("Export") } footer: {
                Text("Keys, secret values and hook commands make the whole file encrypted with a passphrase you type twice. There is no way to recover a lost passphrase.")
            }
            Section("Private keys (off by default)") {
                let keys = backup.keyRecords()
                if keys.isEmpty { Text("No keys").foregroundColor(.secondary) }
                ForEach(keys, id: \.id) { k in
                    Toggle(k.name, isOn: Binding(
                        get: { backup.selection.keyIDs.contains(k.id) && backup.selection.groups.contains(BackupService.GroupID.keys) },
                        set: { on in
                            if on { backup.selection.keyIDs.insert(k.id); backup.selection.groups.insert(BackupService.GroupID.keys) }
                            else {
                                backup.selection.keyIDs.remove(k.id)
                                if backup.selection.keyIDs.isEmpty { backup.selection.groups.remove(BackupService.GroupID.keys) }
                            }
                        }))
                }
            }
            if hasSecrets {
                Section {
                    SecureField("Passphrase (12+ characters)", text: $phrase)
                    SecureField("Passphrase again", text: $phraseAgain)
                    if !phrase.isEmpty, !phraseOK {
                        Text(phrase.count < 12 ? "At least 12 characters." : "The two entries differ.")
                            .font(.footnote).foregroundColor(.orange)
                    }
                }
            }
            Section {
                Button {
                    exporting = true
                    Task {
                        exportURL = await backup.export(servers: serversManager, agents: agentsManager,
                                                        passphrase: hasSecrets ? Data(phrase.utf8) : nil)
                        exporting = false
                    }
                } label: { Label("Create backup file", systemImage: "square.and.arrow.up") }
                    .disabled(!phraseOK || exporting)
                if let url = exportURL {
                    ShareLink(item: url) { Label("Save or share the file", systemImage: "doc.badge.arrow.up") }
                }
            }
        }
    }

    private var importSection: some View {
        Group {
            Section {
                Button { picking = true } label: { Label("Choose a backup file", systemImage: "square.and.arrow.down") }
            } header: { Text("Import") } footer: {
                Text("Nothing changes until you press Apply. “Add new” keeps what you have; “Replace same” overwrites items with the same id. Nothing is ever deleted.")
            }
            if !backup.preview.isEmpty {
                ForEach(backup.preview) { row in
                    Section {
                        Picker("\(row.title) (\(row.count))", selection: Binding(
                            get: { backup.modes[row.id] ?? .skip },
                            set: { backup.modes[row.id] = $0 })) {
                            ForEach(BackupController.ImportMode.allCases) { Text($0.title).tag($0) }
                        }
                        if !row.names.isEmpty {
                            Text(row.names.prefix(8).joined(separator: ", ") + (row.names.count > 8 ? " +\(row.names.count - 8)" : ""))
                                .font(.caption).foregroundColor(.secondary)
                        }
                        ForEach(row.commands, id: \.self) { c in
                            Text(verbatim: c).font(.system(size: 11, design: .monospaced)).foregroundColor(.orange)
                        }
                    }
                }
                if backup.preview.contains(where: { !$0.commands.isEmpty }) {
                    Section {
                        Toggle("I have read these commands; the host will run them", isOn: $backup.commandsConfirmed)
                    }
                }
                Section {
                    Button("Apply") { Task { await backup.apply(servers: serversManager, agents: agentsManager) } }
                        .disabled(!backup.canApply)
                    Button("Cancel", role: .cancel) { backup.reset() }
                }
            }
        }
    }
}
