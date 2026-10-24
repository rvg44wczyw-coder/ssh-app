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
    @State private var showKeyFileImporter = false
    @State private var importPassphrase = ""
    @State private var importNeedsPassphrase = false
    @State private var importTask: Task<Void, Never>? = nil
    @State private var importError: String? = nil
    @State private var keyToDelete: SSHKeyInfo? = nil

    private func startKeyImport() {
        let passphrase = importNeedsPassphrase && !importPassphrase.isEmpty ? Data(importPassphrase.utf8) : nil
        importPassphrase = ""
        importError = nil
        importTask = Task {
            do {
                try await keyStore.importKey(name: newKeyName, privateKey: importText, passphrase: passphrase)
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
        importText = ""; importPassphrase = ""; importNeedsPassphrase = false; importError = nil
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
                    importNeedsPassphrase = (try? privateKeyNeedsPassphrase(armoredKey: text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n")) ?? false
                    importError = nil
                }
                .fileImporter(isPresented: $showKeyFileImporter, allowedContentTypes: [.data, .text, .plainText]) { result in
                    guard case .success(let url) = result else { return }
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    if let data = try? Data(contentsOf: url), data.count < 16_384, let text = String(data: data, encoding: .utf8) {
                        importText = text
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
            let keypair = try generateSshKeypair()
            try KeychainManager.shared.savePrivateKey(keypair.privateKeyOpenssh, overwrite: false)
            try KeychainManager.shared.savePublicKey(keypair.publicKeyOpenssh, overwrite: false)
            publicKeyText = keypair.publicKeyOpenssh
            hasPrivateKey = true
            errorMessage = nil
        } catch {
            errorMessage = "Failed to generate keypair: \(error.localizedDescription)"
        }
    }

    private func forceRegenerateKeys() {
        do {
            let keypair = try generateSshKeypair()
            try KeychainManager.shared.savePrivateKey(keypair.privateKeyOpenssh, overwrite: true)
            try KeychainManager.shared.savePublicKey(keypair.publicKeyOpenssh, overwrite: true)
            publicKeyText = keypair.publicKeyOpenssh
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
