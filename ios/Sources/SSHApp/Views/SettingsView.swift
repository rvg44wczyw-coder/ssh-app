import SwiftUI
import SshCoreBridge

public struct SettingsView: View {
    @ObservedObject public var serversManager: ServerProfilesManager

    @State private var publicKeyText: String = ""
    @State private var hasPrivateKey: Bool = false
    @State private var showCopiedAlert: Bool = false
    @State private var showRegenerateConfirmAlert: Bool = false
    @State private var errorMessage: String? = nil

    @State private var serverToEdit: ServerProfile? = nil
    @State private var showAddServerSheet: Bool = false

    @AppStorage("terminal_font_size") private var terminalFontSize: Double = 13.0

    @Environment(\.dismiss) private var dismiss

    /// The agent profile manager, supplied by the main screen (needs the live session).
    private let agentsPage: AnyView?

    public init(serversManager: ServerProfilesManager, agentsPage: AnyView? = nil) {
        self.serversManager = serversManager
        self.agentsPage = agentsPage
    }

    private enum Page: Hashable { case servers, keys, terminal, agents }

    public var body: some View {
        NavigationStack {
            List {
                NavigationLink(value: Page.servers) { Label("Servers", systemImage: "server.rack") }
                NavigationLink(value: Page.keys) { Label("SSH keys", systemImage: "key.fill") }
                if agentsPage != nil {
                    NavigationLink(value: Page.agents) { Label("Agents", systemImage: "play.rectangle.on.rectangle.fill") }
                }
                NavigationLink(value: Page.terminal) { Label("Terminal", systemImage: "terminal") }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .navigationDestination(for: Page.self) { page in
                switch page {
                case .servers: serversPage
                case .keys: keysPage
                case .terminal: terminalPage
                case .agents: if let agentsPage { agentsPage }
                }
            }
        }
    }

    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false

    private var terminalPage: some View {
        Form {
            Section(
                header: Text("Quick menu"),
                footer: Text("Auto shows the AI Assistant (quick menu tile and the key above the keyboard) only while connected, since it needs the host. Always keeps the tile without a connection (disabled until connected). Never hides it everywhere.")
            ) {
                Picker("AI Assistant", selection: Binding(
                    get: { AiAssistantVisibility.resolved(raw: aiVisibilityRaw, legacyAlways: legacyAlwaysShowAi).rawValue },
                    set: { aiVisibilityRaw = $0 }
                )) {
                    Text("Auto").tag(AiAssistantVisibility.auto.rawValue)
                    Text("Always").tag(AiAssistantVisibility.always.rawValue)
                    Text("Never").tag(AiAssistantVisibility.never.rawValue)
                }
            }
                Section(header: Text("Terminal Font Size")) {
                    HStack {
                        Text("Size: \(Int(terminalFontSize)) pt")
                            .font(.subheadline)
                        Spacer()
                        Button(action: {
                            if terminalFontSize > 8 {
                                terminalFontSize -= 1
                            }
                        }) {
                            Image(systemName: "textformat.size.smaller")
                                .font(.body.bold())
                        }
                        .buttonStyle(BorderlessButtonStyle())

                        Slider(value: $terminalFontSize, in: 8...24, step: 1)
                            .frame(width: 130)

                        Button(action: {
                            if terminalFontSize < 24 {
                                terminalFontSize += 1
                            }
                        }) {
                            Image(systemName: "textformat.size.larger")
                                .font(.body.bold())
                        }
                        .buttonStyle(BorderlessButtonStyle())
                    }
                    .padding(.vertical, 2)
                }

        }
        .navigationTitle("Terminal")
    }

    private var serversPage: some View {
        Form {
                Section(header: Text("SSH Servers"), footer: Text("Tap a server to set it as active. New terminal sessions connect to the active server.")) {
                    if serversManager.profiles.isEmpty {
                        Text("No servers configured")
                            .foregroundColor(.secondary)
                            .italic()
                    } else {
                        ForEach(serversManager.profiles) { server in
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack(spacing: 6) {
                                        Text(server.name)
                                            .font(.headline)
                                        if server.isDefault {
                                            Text("DEFAULT")
                                                .font(.system(size: 9, weight: .bold))
                                                .padding(.horizontal, 5)
                                                .padding(.vertical, 1)
                                                .background(Color.blue.opacity(0.15))
                                                .foregroundColor(.blue)
                                                .cornerRadius(3)
                                        }
                                    }
                                    Text(verbatim: "\(server.username)@\(server.host):\(server.port)")
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }

                                Spacer()

                                if serversManager.activeProfileId == server.id {
                                    Image(systemName: "checkmark.circle.fill")
                                        .foregroundColor(.accentColor)
                                        .font(.title3)
                                }

                                Button(action: {
                                    serverToEdit = server
                                }) {
                                    Image(systemName: "pencil")
                                        .foregroundColor(.secondary)
                                }
                                .buttonStyle(BorderlessButtonStyle())
                                .padding(.leading, 8)
                            }
                            .contentShape(Rectangle())
                            .onTapGesture {
                                serversManager.activeProfileId = server.id
                            }
                        }
                        .onDelete { indexSet in
                            for index in indexSet {
                                let id = serversManager.profiles[index].id
                                serversManager.deleteProfile(id: id)
                            }
                        }
                    }

                    Button(action: {
                        showAddServerSheet = true
                    }) {
                        Label("Add Server...", systemImage: "plus.circle.fill")
                            .font(.subheadline.bold())
                    }
                }

        }
        .navigationTitle("Servers")
            .sheet(isPresented: $showAddServerSheet) {
                AddEditServerSheetView(existingProfile: nil) { newProfile in
                    _ = serversManager.addProfile(
                        name: newProfile.name,
                        host: newProfile.host,
                        port: newProfile.port,
                        username: newProfile.username,
                        isDefault: newProfile.isDefault,
                        keyId: newProfile.keyId
                    )
                }
            }
            .sheet(item: $serverToEdit) { profile in
                AddEditServerSheetView(existingProfile: profile) { updated in
                    serversManager.updateProfile(updated)
                }
            }
    }

    private var keysPage: some View {
        Form {
                Section(
                    header: Text("SSH Key Management (Ed25519)"),
                    footer: Text("Add this public key to ~/.ssh/authorized_keys on all your remote machines to allow passwordless Ed25519 login.")
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
                                HStack {
                                    Image(systemName: "doc.on.doc")
                                    Text("Copy Public Key to Clipboard")
                                }
                                .font(.subheadline)
                                .foregroundColor(.accentColor)
                            }
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
    @State private var keyToDelete: SSHKeyInfo? = nil

    private func usedBy(_ id: String) -> [String] {
        serversManager.profiles.filter { $0.keyId == id }.map(\.name)
    }

    private var additionalKeysSection: some View {
        Section(
            header: Text("More SSH keys"),
            footer: Text("Use a different key per server (pick it in the server's settings). Private keys stay in this device's Keychain; imported keys must be unencrypted OpenSSH Ed25519 (`-----BEGIN OPENSSH PRIVATE KEY-----`).")
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
                    Section(footer: Text("Paste the contents of an unencrypted OpenSSH private key (id_ed25519), or choose the file. The name above is used.")) {
                        TextEditor(text: $importText)
                            .font(.system(size: 11, design: .monospaced))
                            .frame(minHeight: 160)
                            .autocorrectionDisabled()
                        Button("Choose file…") { showKeyFileImporter = true }
                    }
                }
                .navigationTitle("Import key")
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { importingKey = false; importText = "" } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Import") {
                            do {
                                try keyStore.importKey(name: newKeyName, privateKey: importText)
                                newKeyName = ""; importText = ""; importingKey = false; errorMessage = nil
                            } catch { errorMessage = "Import failed: \(error.localizedDescription)"; importingKey = false }
                        }
                        .disabled(importText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
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
