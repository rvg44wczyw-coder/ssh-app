import SwiftUI
import SshCoreBridge

/// Settings -> SSH keys: the default key, more keys, import.
struct KeysSettingsPage: View {
    @ObservedObject var serversManager: ServerProfilesManager
    @State private var publicKeyText: String = ""
    @State private var hasPrivateKey: Bool = false
    @State private var showCopiedAlert: Bool = false
    @State private var showRegenerateConfirmAlert: Bool = false
    @State private var errorMessage: String? = nil

    var body: some View {
        Form {
                Section(
                    header: Text("SSH Key Management (Ed25519)"),
                    footer: Text("Add this public key to ~/.ssh/authorized_keys on each host.")
                ) {
                    if hasPrivateKey {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Public Key:")
                                .font(.caption)
                                .foregroundColor(.secondary)

                            Text(publicKeyText.isEmpty ? "Key generated in Keychain" : publicKeyText)
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(3)
                                .foregroundColor(.primary)

                            Button(action: copyPublicKey) {
                                Label("Copy public key", systemImage: "doc.on.doc")
                            }
                            .buttonStyle(.glass)
                            .padding(.top, 4)
                        }
                        .padding(.vertical, 4)

                        Button("Export private key…") { exportTarget = ExportTarget(keyId: nil, name: "Default key") }

                        Button(role: .destructive, action: { showRegenerateConfirmAlert = true }) {
                            Text("Regenerate New Keypair")
                        }
                    } else {
                        Button(action: generateNewKeys) {
                            HStack {
                                Image(systemName: "key.fill")
                                Text("Generate Ed25519 Keypair")
                            }
                            .font(.headline)
                        }
                    }
                }

                additionalKeysSection

                if let err = errorMessage {
                    Section {
                        Text(err)
                            .foregroundColor(.red)
                            .font(.caption)
                    }
                }
        }
        .navigationTitle("SSH keys")
        .sheet(item: $exportTarget) { KeyExportSheet(target: $0) }
        .onAppear(perform: loadCurrentKeyStatus)
            .alert("Copied!", isPresented: $showCopiedAlert) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("OpenSSH public key copied to clipboard. Paste into ~/.ssh/authorized_keys on your remote host.")
            }
            .alert("Regenerate SSH Keypair?", isPresented: $showRegenerateConfirmAlert) {
                Button("Cancel", role: .cancel) {}
                Button("Regenerate & Overwrite", role: .destructive) {
                    forceRegenerateKeys()
                }
            } message: {
                Text("Warning: Regenerating will permanently replace your current SSH key. You will lose access to all remote servers until you update ~/.ssh/authorized_keys with the new key.")
            }
    }

    @ObservedObject private var keyStore = SSHKeyStore.shared
    @State private var newKeyName = ""
    @State private var importingKey = false
    @State private var importText = ""
    /// A key file read as bytes; it never becomes a `String`.
    @State private var importFileData: Data? = nil
    @State private var showKeyFileImporter = false
    @State private var importPassphrase = ""
    @State private var importNeedsPassphrase = false
    @State private var importTask: Task<Void, Never>? = nil
    @State private var importError: String? = nil
    @State private var keyToDelete: SSHKeyInfo? = nil
    @State private var exportTarget: ExportTarget? = nil

    private func startKeyImport() {
        let passphrase = importNeedsPassphrase && !importPassphrase.isEmpty ? Data(importPassphrase.utf8) : nil
        importPassphrase = ""
        importError = nil
        let armored = importFileData ?? Data((importText.trimmingCharacters(in: .whitespacesAndNewlines) + "\n").utf8)
        importText = ""; importFileData = nil
        importTask = Task {
            do {
                try await keyStore.importKey(name: newKeyName, privateKey: armored, passphrase: passphrase)
                newKeyName = ""; errorMessage = nil
                endKeyImport()
            } catch is CancellationError {
                importTask = nil
            } catch SshCoreError.KeyPassphraseRequired {
                importError = "This key needs its passphrase."; importTask = nil
            } catch SshCoreError.KeyPassphraseWrong {
                importError = "That passphrase does not unlock the key."; importTask = nil
            } catch {
                importError = "Import failed: \(error.localizedDescription)"; importTask = nil
            }
        }
    }

    private func endKeyImport() {
        importTask?.cancel()
        importTask = nil
        importingKey = false
        importText = ""; importFileData = nil; importPassphrase = ""; importNeedsPassphrase = false; importError = nil
    }

    private func usedBy(_ id: String) -> [String] {
        serversManager.profiles.filter { $0.keyId == id }.map(\.name)
    }

    private var additionalKeysSection: some View {
        Section(
            header: Text("More SSH keys"),
            footer: Text("Pick a key per server in the server's settings. Private keys stay in this device's Keychain.")
        ) {
            ForEach(keyStore.extra) { k in
                VStack(alignment: .leading, spacing: 4) {
                    Text(k.name).font(.headline)
                    let used = usedBy(k.id)
                    Text(used.isEmpty ? "Not used by any server" : "Used by: " + used.joined(separator: ", "))
                        .font(.caption).foregroundColor(.secondary)
                    if let pub = KeychainManager.shared.getOrDerivePublicKey(id: k.id) {
                        Text(pub).font(.system(size: 10, design: .monospaced)).lineLimit(2).foregroundColor(.secondary)
                        HStack(spacing: 16) {
                            Button("Copy public key") {
                                KeychainManager.shared.copyPublicKeyToClipboard(pub)
                                showCopiedAlert = true
                            }
                            Button("Export…") { exportTarget = ExportTarget(keyId: k.id, name: k.name) }
                            Button("Delete", role: .destructive) { keyToDelete = k }
                        }
                        .font(.caption).buttonStyle(.borderless)
                    }
                }
                .padding(.vertical, 2)
            }
            TextField("Name for a new key (e.g. work)", text: $newKeyName)
                .autocorrectionDisabled()
            Button {
                do {
                    try keyStore.generate(name: newKeyName)
                    newKeyName = ""
                    errorMessage = nil
                } catch { errorMessage = "Failed to generate key: \(error.localizedDescription)" }
            } label: { Label("Generate new key", systemImage: "key.fill") }
            Button { importingKey = true } label: { Label("Import private key…", systemImage: "square.and.arrow.down") }
        }
        .sheet(isPresented: $importingKey) {
            NavigationStack {
                Form {
                    Section(footer: Text(importError ?? "Paste the contents of an OpenSSH private key (id_ed25519), or choose the file. The name above is used. A key with a passphrase asks for it.")) {
                        TextEditor(text: $importText)
                            .font(.system(size: 11, design: .monospaced))
                            .frame(minHeight: 160)
                            .autocorrectionDisabled()
                        Button("Choose file…") { showKeyFileImporter = true }
                    }
                    if importNeedsPassphrase {
                        Section(footer: Text("The passphrase is used once to unlock the key and is not stored.")) {
                            SecureField("Key passphrase", text: $importPassphrase)
                                .textContentType(.password)
                        }
                    }
                    if importTask != nil {
                        Section { HStack { ProgressView(); Text("Unlocking the key…") } }
                    }
                }
                .navigationTitle("Import key")
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { endKeyImport() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Import") { startKeyImport() }
                            .disabled(importText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || importTask != nil)
                    }
                }
                .onChange(of: importText) { _, text in
                    guard importFileData == nil else { return }
                    importNeedsPassphrase = (try? keyFileNeedsPassphrase(armored: Data((text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n").utf8))) ?? false
                    importError = nil
                }
                .fileImporter(isPresented: $showKeyFileImporter, allowedContentTypes: [.data, .text, .plainText]) { result in
                    guard case .success(let url) = result else { return }
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    if let data = try? Data(contentsOf: url), data.count < 16_384 {
                        importFileData = data
                        importText = "(\(url.lastPathComponent))"
                        importNeedsPassphrase = (try? keyFileNeedsPassphrase(armored: data)) ?? false
                        importError = nil
                    } else {
                        errorMessage = "That file is not a readable key (too large or not text)."
                    }
                }
            }
            .interactiveDismissDisabled()
        }
        .alert("Delete key?", isPresented: Binding(get: { keyToDelete != nil }, set: { if !$0 { keyToDelete = nil } })) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                if let k = keyToDelete {
                    for s in serversManager.profiles where s.keyId == k.id {
                        var u = s; u.keyId = nil; serversManager.updateProfile(u)
                    }
                    keyStore.delete(id: k.id)
                }
            }
        } message: {
            Text("The private key is removed from this device for good. Servers that used it fall back to the default key.")
        }
    }

    private func loadCurrentKeyStatus() {
        if let key = KeychainManager.shared.getOrDerivePublicKey(), !key.isEmpty {
            publicKeyText = key
            hasPrivateKey = true
        } else {
            hasPrivateKey = false
        }
    }

    private func generateNewKeys() {
        guard !KeychainManager.shared.hasPrivateKey() else {
            errorMessage = "A keypair already exists. Overwrite requires explicit confirmation."
            return
        }
        do {
            let info = try KeychainManager.shared.generateKey(overwrite: false)
            publicKeyText = info.publicKeyOpenssh
            hasPrivateKey = true
            errorMessage = nil
        } catch {
            errorMessage = "Failed to generate keypair: \(error.localizedDescription)"
        }
    }

    private func forceRegenerateKeys() {
        do {
            let info = try KeychainManager.shared.generateKey(overwrite: true)
            publicKeyText = info.publicKeyOpenssh
            hasPrivateKey = true
            errorMessage = nil
        } catch {
            errorMessage = "Failed to regenerate keypair: \(error.localizedDescription)"
        }
    }

    private func copyPublicKey() {
        #if canImport(UIKit)
        UIPasteboard.general.string = publicKeyText
        showCopiedAlert = true
        #endif
    }
}


struct ExportTarget: Identifiable {
    var keyId: String?
    var name: String
    var id: String { keyId ?? "default" }
}

/// Exporting a private key in the clear: three separate confirmations, then the core hands the key
/// over as bytes that go straight into a protected temporary file for the share sheet. The key text
/// is never shown and never becomes a Swift `String`.
struct KeyExportSheet: View {
    let target: ExportTarget
    @Environment(\.dismiss) private var dismiss
    @State private var understandsAccess = false
    @State private var understandsExposure = false
    @State private var typed = ""
    @State private var fileURL: URL?
    @State private var error: String?

    private var ready: Bool { understandsAccess && understandsExposure && typed == "EXPORT" }

    var body: some View {
        NavigationStack {
            Form {
                Section(footer: Text("The key is exported as a file, not shown. Delete the copy when you are done.")) {
                    Toggle("Whoever has this key can log in to every server that trusts it.", isOn: $understandsAccess)
                    Toggle("A copy in a file, the clipboard or a backup can be read by other apps and people.", isOn: $understandsExposure)
                    TextField("Type EXPORT to confirm", text: $typed)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                }
                if let error { Section { Text(error).foregroundColor(.red).font(.caption) } }
                Section {
                    if let fileURL {
                        ShareLink(item: fileURL) { Label("Share \(target.name)", systemImage: "square.and.arrow.up") }
                    } else {
                        Button("Export private key") { Task { await export() } }.disabled(!ready)
                    }
                }
            }
            .navigationTitle("Export \(target.name)")
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { close() } } }
        }
        .interactiveDismissDisabled(fileURL != nil)
        .onDisappear { remove() }
    }

    private func export() async {
        guard ready, await AppLock.shared.confirmOwner(reason: "Confirm to export a private key") else {
            error = ready ? "Not confirmed" : nil
            return
        }
        let acks = ExportAcks(understandsAccess: understandsAccess, understandsExposure: understandsExposure,
                              typedConfirmation: typed)
        do {
            var bytes = try keyExportPlain(key: KeychainManager.shared.keyRef(target.keyId), acks: acks)
            defer { bytes.resetBytes(in: 0..<bytes.count) }
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("keyexport-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.complete])
            let url = dir.appendingPathComponent("id_ed25519")
            try bytes.write(to: url, options: [.atomic, .completeFileProtection])
            fileURL = url
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func remove() {
        if let fileURL { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }
        fileURL = nil
    }

    private func close() { remove(); dismiss() }
}
