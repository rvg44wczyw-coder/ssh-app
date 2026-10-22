import SwiftUI
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#endif

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
    @State private var muxPreference: MultiplexerPreference = .auto
    @ObservedObject private var keys = SSHKeyStore.shared
    @State private var showPairing = false
    @State private var pairedHostKey: (host: String, port: UInt16, fingerprint: String)? = nil
    @State private var pasteNote: String? = nil

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
        if let p = existingProfile {
            _muxPreference = State(initialValue: HostTargetCache.multiplexerPreference(
                forKey: HostTargetCache.key(host: p.host, port: p.port, username: p.username)))
        }
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
                    header: Text("Set up this phone on the host"),
                    footer: Text(pasteNote ?? "Either log in with a password once (the password is used for this single step and never stored), or copy a command and run it on the host yourself. Both add this phone's key to ~/.ssh/authorized_keys.")
                ) {
                    Button("Log in once with a password…") { showPairing = true }
                        .disabled(host.trimmingCharacters(in: .whitespaces).isEmpty || username.trimmingCharacters(in: .whitespaces).isEmpty)
                    Button("Copy the command to run on the host") { copyPairingCommand() }
                    if let paired = pairedHostKey, paired.host == host.trimmingCharacters(in: .whitespacesAndNewlines) {
                        Label("Key added; host key pinned", systemImage: "checkmark.seal")
                            .font(.caption).foregroundColor(.green)
                    }
                }

                Section(
                    header: Text("Sessions"),
                    footer: Text("What keeps terminal sessions alive on this host. With both installed, Auto uses tmux. Zellij tabs are terminals only: no agent feed, screen watch or answering agents from the phone.")
                ) {
                    Picker("Session program", selection: $muxPreference) {
                        Text("Auto").tag(MultiplexerPreference.auto)
                        Text("Prefer tmux").tag(MultiplexerPreference.preferTmux)
                        Text("Prefer Zellij").tag(MultiplexerPreference.preferZellij)
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
        .sheet(isPresented: $showPairing) {
            PairHostSheet(
                host: host.trimmingCharacters(in: .whitespacesAndNewlines),
                port: UInt16(clamping: port),
                username: username.trimmingCharacters(in: .whitespacesAndNewlines),
                keyId: keyId
            ) { fingerprint in
                pairedHostKey = (host.trimmingCharacters(in: .whitespacesAndNewlines), UInt16(clamping: port), fingerprint)
            }
        }
    }

    private func copyPairingCommand() {
        guard let pub = KeychainManager.shared.getOrDerivePublicKey(id: keyId), !pub.isEmpty else {
            pasteNote = "This phone has no key yet: create one in Settings first."
            return
        }
        do {
            let command = try pairingPasteCommand(publicKey: pub, deviceName: PairHostSheet.defaultDeviceName())
            #if canImport(UIKit)
            UIPasteboard.general.string = command
            #endif
            pasteNote = "Copied. Paste it into a terminal on the host: it adds this phone's key to ~/.ssh/authorized_keys (once) and prints what it did."
        } catch {
            pasteNote = "Could not build the command: \(error.localizedDescription)"
        }
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

        HostTargetCache.setMultiplexerPreference(
            muxPreference, forKey: HostTargetCache.key(host: trimmedHost, port: UInt16(clamping: port), username: trimmedUser))
        onSave(profile)
        dismiss()
    }

    /// The pin survives edits that do not change which machine it identifies.
    private func keptHostKey(host trimmedHost: String) -> String? {
        if let paired = pairedHostKey, paired.host == trimmedHost, paired.port == UInt16(clamping: port) {
            return paired.fingerprint
        }
        guard let old = existingProfile, !forgetHostKey,
              old.host == trimmedHost, old.port == UInt16(clamping: port) else { return nil }
        return old.hostKeyFingerprint
    }
}

/// One-time password login that puts this phone's key on a new host (feature 44). The host key is
/// read first and shown; only after the user trusts it is the password sent. The password lives in
/// this view's state for the duration of the step and is cleared as soon as the call returns.
struct PairHostSheet: View {
    let host: String
    let port: UInt16
    let username: String
    let keyId: String?
    let onPinned: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var fingerprint: String?
    @State private var password = ""
    @State private var deviceName = PairHostSheet.defaultDeviceName()
    @State private var busy = false
    @State private var message: String?
    @State private var done = false

    /// The phone's name reduced to what the host accepts in a key comment.
    static func defaultDeviceName() -> String {
        #if canImport(UIKit)
        return sanitizedDeviceName(UIDevice.current.name)
        #else
        return "iPhone"
        #endif
    }

    static func sanitizedDeviceName(_ raw: String) -> String {
        let allowed = raw.filter { $0.isASCII && ($0.isLetter || $0.isNumber || " ._-".contains($0)) }
        let trimmed = String(allowed.prefix(40)).trimmingCharacters(in: .whitespaces)
        return trimmed.isEmpty ? "iPhone" : trimmed
    }

    var body: some View {
        NavigationView {
            Form {
                Section(header: Text("Host"), footer: Text("\(username)@\(host):\(port)")) {
                    if let fingerprint {
                        Text(verbatim: fingerprint).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                        Text("Compare it with the host: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub. Continue only if they match.")
                            .font(.caption).foregroundColor(.secondary)
                    } else {
                        Button("Read the host key") { Task { await readKey() } }.disabled(busy)
                    }
                }
                if fingerprint != nil, !done {
                    Section(
                        header: Text("Login"),
                        footer: Text("The password is sent once over the encrypted connection to this host and is not stored. Afterwards this phone signs in with its key.")
                    ) {
                        SecureField("Password of \(username)", text: $password)
                        TextField("Name of this phone", text: $deviceName)
                            .autocorrectionDisabled()
                        Button("Trust this host and add my key") { Task { await pair() } }
                            .disabled(busy || password.isEmpty || deviceName.isEmpty)
                    }
                }
                if busy { Section { ProgressView() } }
                if let message {
                    Section { Text(message).font(.caption).foregroundColor(done ? .green : .red) }
                }
            }
            .navigationTitle("Add this phone")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(done ? "Done" : "Cancel") { dismiss() } }
            }
        }
        .interactiveDismissDisabled(busy)
        .onDisappear { password = "" }
    }

    private func readKey() async {
        busy = true; message = nil
        defer { busy = false }
        do {
            fingerprint = try await probeServerHostKey(host: host, port: port)
        } catch {
            message = "Could not reach the host: \(error.localizedDescription)"
        }
    }

    private func pair() async {
        guard let fingerprint else { return }
        guard let pub = KeychainManager.shared.getOrDerivePublicKey(id: keyId), !pub.isEmpty else {
            message = "This phone has no key yet: create one in Settings first."
            return
        }
        busy = true; message = nil
        var typed = password
        password = ""
        defer { busy = false; typed = "" }
        do {
            let state = try await pairWithPassword(
                host: host, port: port, username: username, password: typed,
                hostKey: fingerprint, publicKey: pub, deviceName: deviceName)
            switch state {
            case .added:
                message = "Done: this phone's key was added. The host key is pinned."
                done = true
            case .alreadyPresent:
                message = "This phone's key was already on the host. The host key is pinned."
                done = true
            case .refused:
                message = "The host refused: ~/.ssh or authorized_keys is a symlink. Nothing was changed."
            case .failed:
                message = "The host did not confirm. Nothing was verified; check ~/.ssh on the host."
            }
            if done { onPinned(fingerprint) }
        } catch {
            message = "Failed: \(error.localizedDescription)"
        }
    }
}
