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

    public init(serversManager: ServerProfilesManager) {
        self.serversManager = serversManager
    }

    public var body: some View {
        NavigationStack {
            Form {
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
                                    Text("\(server.username)@\(server.host):\(server.port)")
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

                if let err = errorMessage {
                    Section {
                        Text(err)
                            .foregroundColor(.red)
                            .font(.caption)
                    }
                }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .onAppear(perform: loadCurrentKeyStatus)
            .sheet(isPresented: $showAddServerSheet) {
                AddEditServerSheetView(existingProfile: nil) { newProfile in
                    _ = serversManager.addProfile(
                        name: newProfile.name,
                        host: newProfile.host,
                        port: newProfile.port,
                        username: newProfile.username,
                        isDefault: newProfile.isDefault
                    )
                }
            }
            .sheet(item: $serverToEdit) { profile in
                AddEditServerSheetView(existingProfile: profile) { updated in
                    serversManager.updateProfile(updated)
                }
            }
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
