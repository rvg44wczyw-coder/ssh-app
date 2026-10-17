import SwiftUI

public struct AddEditServerSheetView: View {
    @Environment(\.dismiss) private var dismiss

    private let existingProfile: ServerProfile?
    private let onSave: (ServerProfile) -> Void

    @State private var name: String = ""
    @State private var host: String = ""
    @State private var port: Int = 22
    @State private var username: String = ""
    @State private var isDefault: Bool = false
    @State private var errorMessage: String? = nil
    @State private var forgetHostKey = false
    @State private var keyId: String? = nil
    @State private var showInFiles: Bool = true
    @State private var filesFolder: String = ""
    @ObservedObject private var keys = SSHKeyStore.shared

    public init(
        existingProfile: ServerProfile? = nil,
        onSave: @escaping (ServerProfile) -> Void
    ) {
        self.existingProfile = existingProfile
        self.onSave = onSave
        _name = State(initialValue: existingProfile?.name ?? "")
        _host = State(initialValue: existingProfile?.host ?? "")
        _port = State(initialValue: Int(existingProfile?.port ?? 22))
        _username = State(initialValue: existingProfile?.username ?? "")
        _isDefault = State(initialValue: existingProfile?.isDefault ?? false)
        _keyId = State(initialValue: existingProfile?.keyId)
        _showInFiles = State(initialValue: existingProfile?.showInFiles ?? true)
        _filesFolder = State(initialValue: existingProfile?.filesFolder ?? "")
    }

    public var body: some View {
        NavigationView {
            Form {
                Section(header: Text("Server Details")) {
                    TextField("Server Name (e.g. MacBook Pro)", text: $name)

                    TextField("Host / IP (e.g. 100.x.y.z)", text: $host)
                        #if os(iOS)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                        #endif

                    TextField("SSH Username", text: $username)
                        #if os(iOS)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                        #endif

                    HStack {
                        Text("Port")
                        Spacer()
                        TextField("22", value: $port, formatter: NumberFormatter())
                            #if os(iOS)
                            .keyboardType(.numberPad)
                            #endif
                            .multilineTextAlignment(.trailing)
                    }
                }

                if let fingerprint = existingProfile?.hostKeyFingerprint {
                    Section(
                        header: Text("Host key"),
                        footer: Text("Pinned on the first connection. If the server's key changes (for example after a reinstall) connections are refused until you forget it here. Changing the host or port also forgets it.")
                    ) {
                        Text(verbatim: forgetHostKey ? "Will be pinned again on the next connection" : fingerprint)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundColor(forgetHostKey ? .secondary : .primary)
                        if !forgetHostKey {
                            Button("Forget pinned key", role: .destructive) { forgetHostKey = true }
                        }
                    }
                }

                Section(header: Text("SSH key"), footer: Text("The key this server accepts. Add its public key to ~/.ssh/authorized_keys on that host. Manage keys in Settings.")) {
                    Picker("Key", selection: $keyId) {
                        Text("Default key").tag(String?.none)
                        ForEach(keys.extra) { k in Text(k.name).tag(String?.some(k.id)) }
                    }
                }

                Section(
                    header: Text("Files app"),
                    footer: Text("Shows this server as a location in the Files app: browse folders and save files to the phone. It appears after the first successful connection (the host key must be pinned). The root is the home folder unless you set another one, as an absolute path, e.g. /Users/me/Terminals.")
                ) {
                    Toggle("Show in Files", isOn: $showInFiles)
                    if showInFiles {
                        TextField("Root folder (absolute path, optional)", text: $filesFolder)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.system(size: 14, design: .monospaced))
                    }
                }

                Section {
                    Toggle("Default Server", isOn: $isDefault)
                }

                if let error = errorMessage {
                    Section {
                        Text(error)
                            .foregroundColor(.red)
                            .font(.caption)
                    }
                }
            }
            .navigationTitle(existingProfile == nil ? "Add Server" : "Edit Server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        save()
                    }
                    .font(.headline)
                }
            }
        }
        .interactiveDismissDisabled()
    }

    private func save() {
        let trimmedHost = host.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedUser = username.trimmingCharacters(in: .whitespacesAndNewlines)
        var trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !trimmedHost.isEmpty else {
            errorMessage = "Host address is required."
            return
        }
        guard !trimmedUser.isEmpty else {
            errorMessage = "Username is required."
            return
        }
        if trimmedName.isEmpty {
            trimmedName = trimmedHost
        }

        let profile = ServerProfile(
            id: existingProfile?.id ?? UUID(),
            name: trimmedName,
            host: trimmedHost,
            port: UInt16(clamping: port),
            username: trimmedUser,
            isDefault: isDefault,
            hostKeyFingerprint: keptHostKey(host: trimmedHost),
            keyId: keyId,
            showInFiles: showInFiles ? nil : false,
            filesFolder: filesFolder.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : filesFolder.trimmingCharacters(in: .whitespacesAndNewlines)
        )

        onSave(profile)
        dismiss()
    }

    /// The pin survives edits that do not change which machine it identifies.
    private func keptHostKey(host trimmedHost: String) -> String? {
        guard let old = existingProfile, !forgetHostKey,
              old.host == trimmedHost, old.port == UInt16(clamping: port) else { return nil }
        return old.hostKeyFingerprint
    }
}
