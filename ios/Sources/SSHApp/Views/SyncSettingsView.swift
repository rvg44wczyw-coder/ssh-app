import SwiftUI
import SshCoreBridge

/// iCloud sync and the encrypted vault (feature 56). Everything is off until the owner turns it on;
/// the vault is what lets private keys and secret values travel at all.
struct SyncSettingsView: View {
    @ObservedObject private var sync = VaultSync.shared
    @ObservedObject private var vault = EncryptedVault.shared
    @AppStorage(VaultSync.enabledKey) private var enabled = false
    @AppStorage(VaultSync.noticeKey) private var notice = false
    @AppStorage(VaultSync.serversKey) private var syncServers = true
    @AppStorage(VaultSync.fingerprintsKey) private var syncFingerprints = false
    @AppStorage(VaultSync.profilesKey) private var syncProfiles = true
    @AppStorage(VaultSync.snippetsKey) private var syncSnippets = true
    @AppStorage(VaultSync.quickCommandsKey) private var syncQuick = true
    @AppStorage(VaultSync.appearanceKey) private var syncAppearance = true
    @AppStorage(VaultSync.secretKeysKey) private var syncKeys = false
    @AppStorage(VaultSync.secretEnvKey) private var syncEnv = false

    @State private var confirmEnable = false
    @State private var confirmRemove = false
    @State private var confirmVaultOff = false
    @State private var passphraseSheet: PassphraseSheet.Mode?
    @State private var joinPhrase = ""
    @State private var joinError: String?
    @State private var keyError: String?
    @State private var refresh = 0

    var body: some View {
        Form {
            if notice { noticeSection }
            masterSection
            if enabled {
                categoriesSection
                vaultSection
                if vault.state == .ready { secretsSection }
                statusSection
            }
        }
        .navigationTitle("iCloud sync")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { notice = false; vault.refresh() }
        .sheet(item: $passphraseSheet) { mode in PassphraseSheet(mode: mode) }
        .confirmationDialog("Turn on iCloud sync?", isPresented: $confirmEnable, titleVisibility: .visible) {
            Button("Turn on") { enabled = true; sync.sync() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Server names and addresses, launch profiles and snippets will be shared with your other devices through iCloud Keychain. Private keys, secret values and host fingerprints stay on this device unless you turn them on below.")
        }
        .confirmationDialog("Remove everything from iCloud?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove and turn sync off", role: .destructive) { sync.removeFromICloud() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The synced lists and sealed secrets are deleted from iCloud. Nothing on this device is deleted; other devices keep their own copies.")
        }
        .confirmationDialog("Turn the vault off?", isPresented: $confirmVaultOff, titleVisibility: .visible) {
            Button("Turn off", role: .destructive) { vault.turnOff() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The vault and everything sealed in it are deleted from iCloud. Keys and values on your devices stay.")
        }
    }

    private var noticeSection: some View {
        Section {
            Label("An update turned iCloud sync off. Turn it on here if you still want it.", systemImage: "info.circle")
                .font(.footnote)
        }
    }

    private var masterSection: some View {
        Section(footer: Text("Off by default. You choose what is shared.")) {
            Toggle("iCloud sync", isOn: Binding(get: { enabled }, set: { on in
                if on { confirmEnable = true } else { enabled = false }
            }))
        }
    }

    private var categoriesSection: some View {
        Section {
            DisclosureGroup("What to sync") {
                Toggle("Servers", isOn: $syncServers)
                Toggle("Launch profiles", isOn: $syncProfiles)
                Toggle("Snippets", isOn: $syncSnippets)
                Toggle("Quick commands", isOn: $syncQuick)
                Toggle("Accent colour", isOn: $syncAppearance)
                Toggle("Pinned host fingerprints", isOn: $syncFingerprints).disabled(!syncServers)
                Text("Sharing a fingerprint tells the other device to trust the same host key.")
                    .font(.caption).foregroundColor(.secondary)
            }
            NavigationLink("What will be sent") { sentList }
        }
    }

    private var sentList: some View {
        List {
            ForEach(sync.outgoingSummary(), id: \.title) { row in
                LabeledContent(row.title, value: "\(row.count)")
            }
        }
        .navigationTitle("What will be sent")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var vaultSection: some View {
        Section(footer: vaultFooter) {
            DisclosureGroup("Encrypted vault") {
                switch vault.state {
                case .off:
                    Text("A vault protects private keys and secret values with a passphrase only you know. Without it they cannot be shared.")
                        .font(.caption).foregroundColor(.secondary)
                    Button("Set up vault…") { passphraseSheet = .create }
                case .needsJoin:
                    Text(vault.replaced
                         ? "The vault was reset or replaced on another device. Unlock the new one, or reset it."
                         : "This iCloud account has a vault. Enter its passphrase to use it here.")
                        .font(.caption).foregroundColor(vault.replaced ? .orange : .secondary)
                    SecureField("Vault passphrase", text: $joinPhrase)
                    if let joinError { Text(joinError).font(.caption).foregroundColor(.red) }
                    Button("Unlock") { join() }.disabled(joinPhrase.isEmpty)
                    Button("Forgot the passphrase? Reset vault…", role: .destructive) { passphraseSheet = .reset }
                case .removed:
                    Text("The vault is gone from iCloud. It was removed on another device, or deleted. Sync is paused.")
                        .font(.caption).foregroundColor(.orange)
                    Button("Set up a new vault…") { vault.forget(); passphraseSheet = .create }
                    Button("Forget it on this device", role: .destructive) { vault.forget() }
                case .ready:
                    if let created = vault.createdAt {
                        LabeledContent("Created", value: created.formatted(date: .abbreviated, time: .omitted))
                    }
                    Button("Change passphrase…") { passphraseSheet = .change }
                    Button("Reset vault…") { passphraseSheet = .reset }
                    Button("Turn off vault", role: .destructive) { confirmVaultOff = true }
                }
            }
        }
    }

    private var vaultFooter: some View {
        Text(vault.state == .ready
             ? "If you lose the passphrase the vault cannot be recovered. Reset it if a device is lost."
             : "Without a vault the shared lists are protected only by iCloud Keychain.")
    }

    private var secretsSection: some View {
        let offers = sync.secretOffers()
        return Section(footer: Text(keyError ?? "A private key leaves this phone when you turn it on. Anyone who has your iCloud account and the vault passphrase can use it.")) {
            DisclosureGroup("Private keys") {
                Toggle("Share private keys", isOn: $syncKeys)
                if syncKeys {
                    ForEach(offers.keys, id: \.id) { item in secretRow(item, keyItem: true) }
                    if offers.keys.isEmpty { Text("No keys on this device.").font(.caption).foregroundColor(.secondary) }
                }
            }
            DisclosureGroup("Secret values") {
                Toggle("Share secret values", isOn: $syncEnv)
                if syncEnv {
                    ForEach(offers.env, id: \.id) { item in secretRow(item, keyItem: false) }
                    if offers.env.isEmpty { Text("No secret values in your profiles.").font(.caption).foregroundColor(.secondary) }
                }
            }
        }
        .id(refresh)
    }

    private func secretRow(_ item: SecretItem, keyItem: Bool) -> some View {
        Toggle(item.label, isOn: Binding(get: { sync.secretEnabled(item.id) }, set: { on in
            guard on, keyItem else { sync.setSecret(item.id, enabled: on); refresh += 1; return }
            Task {
                if await Biometrics.authenticate(reason: "Confirm to share this private key through iCloud") {
                    keyError = nil
                    sync.setSecret(item.id, enabled: true)
                } else {
                    keyError = "Face ID or the passcode is needed to share a key."
                }
                refresh += 1
            }
        }))
    }

    private var statusSection: some View {
        Section {
            Button("Sync now") { sync.sync() }
            if let last = sync.lastSync {
                Text("Last synced \(last.formatted(date: .omitted, time: .shortened))")
                    .font(.caption).foregroundColor(.secondary)
            }
            if let err = sync.lastError { Text(err).font(.caption).foregroundColor(.orange) }
            if sync.rejectedEntries > 0 || sync.secretsRejected > 0 {
                Text("\(sync.rejectedEntries + sync.secretsRejected) entries in iCloud were ignored: they were not made with your vault key.")
                    .font(.caption).foregroundColor(.orange)
            }
            if !sync.fingerprintConflicts.isEmpty {
                Text("The host key differs from the one pinned on another device for: \(sync.fingerprintConflicts.joined(separator: ", ")). This device keeps its own pin; check the server before trusting either.")
                    .font(.caption).foregroundColor(.red)
            }
            Button("Remove everything from iCloud…", role: .destructive) { confirmRemove = true }
        }
    }

    private func join() {
        do {
            try vault.join(passphrase: joinPhrase)
            joinPhrase = ""
            joinError = nil
            sync.sync()
        } catch {
            joinError = error.localizedDescription
        }
    }
}

/// Typed twice, by hand. There is no generator and no recovery.
struct PassphraseSheet: View {
    enum Mode: String, Identifiable {
        case create, change, reset
        var id: String { rawValue }
    }

    let mode: Mode
    @Environment(\.dismiss) private var dismiss
    @State private var old = ""
    @State private var phrase = ""
    @State private var confirm = ""
    @State private var error: String?

    private var title: String {
        switch mode {
        case .create: return "Set up vault"
        case .change: return "Change passphrase"
        case .reset: return "Reset vault"
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section(footer: Text(footer)) {
                    if mode == .change { SecureField("Current passphrase", text: $old) }
                    SecureField("New passphrase", text: $phrase)
                    SecureField("Repeat passphrase", text: $confirm)
                }
                let check = passphraseStrength(passphrase: Data(phrase.utf8))
                if !phrase.isEmpty, !check.warnings.isEmpty {
                    Section { ForEach(check.warnings, id: \.self) { Text($0).font(.caption).foregroundColor(.orange) } }
                }
                if let error { Section { Text(error).font(.caption).foregroundColor(.red) } }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(mode == .reset ? "Reset" : "Save") { save() }
                        .disabled(phrase.isEmpty || confirm.isEmpty)
                }
            }
        }
        .interactiveDismissDisabled()
    }

    private var footer: String {
        switch mode {
        case .create: return "At least 12 characters, typed by you. If you lose it the vault cannot be recovered."
        case .change: return "Entries stay valid; only the wrapping of the vault key changes."
        case .reset: return "A new vault key replaces the old one and everything sealed with it is deleted from iCloud. Other devices must unlock the new vault."
        }
    }

    private func save() {
        Task {
            if mode != .change, !(await Biometrics.authenticate(reason: "Confirm to change the vault")) {
                error = "Face ID or the passcode is needed."
                return
            }
            do {
                switch mode {
                case .create: try EncryptedVault.shared.create(passphrase: phrase, confirm: confirm)
                case .change: try EncryptedVault.shared.changePassphrase(old: old, new: phrase, confirm: confirm)
                case .reset: try EncryptedVault.shared.reset(passphrase: phrase, confirm: confirm)
                }
                old = ""; phrase = ""; confirm = ""
                VaultSync.shared.sync()
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
