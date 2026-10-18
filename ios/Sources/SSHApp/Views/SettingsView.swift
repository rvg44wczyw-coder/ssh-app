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
    @State private var showPaywall: Bool = false
    @ObservedObject private var subs = SubscriptionManager.shared

    @AppStorage("terminal_font_size") private var terminalFontSize: Double = 13.0

    @Environment(\.dismiss) private var dismiss

    /// The agent profile manager, supplied by the main screen (needs the live session).
    private let agentsPage: AnyView?

    public init(serversManager: ServerProfilesManager, agentsPage: AnyView? = nil) {
        self.serversManager = serversManager
        self.agentsPage = agentsPage
    }

    private enum Page: Hashable { case servers, keys, terminal, appearance, agents, subscription }

    public var body: some View {
        NavigationStack {
            List {
                if Monetization.compiledIn {
                    NavigationLink(value: Page.subscription) {
                        HStack {
                            Label("SSH-App Pro", systemImage: "star.circle.fill")
                            Spacer()
                            if subs.paymentsEnabled {
                                Text(subs.isPro ? "Active" : "Free").foregroundColor(.secondary)
                            } else {
                                Text("Off").foregroundColor(.secondary)
                            }
                        }
                    }
                }
                NavigationLink(value: Page.servers) { Label("Servers", systemImage: "server.rack") }
                NavigationLink(value: Page.keys) { Label("SSH keys", systemImage: "key.fill") }
                if agentsPage != nil {
                    NavigationLink(value: Page.agents) { Label("Agents", systemImage: "play.rectangle.on.rectangle.fill") }
                }
                NavigationLink(value: Page.terminal) { Label("Terminal", systemImage: "terminal") }
                NavigationLink(value: Page.appearance) { Label("Appearance", systemImage: "paintpalette") }
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
                case .appearance: appearancePage
                case .agents: if let agentsPage { agentsPage }
                case .subscription: PaywallView(embedded: true)
                }
            }
        }
    }

    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false

    @AppStorage(AppAccent.storageKey) private var accentRaw: String = AppAccent.blue.rawValue
    @AppStorage(TerminalFolder.storageKey) private var terminalFolder: String = TerminalFolder.fallback
    @State private var schemeRefresh = 0
    @AppStorage("usage_strip_enabled") private var usageStripEnabled: Bool = true

    @State private var iconChoice: AppIconChoice = .standard
    @State private var editingScheme: TerminalScheme?
    @State private var iconError: String?

    private func setIcon(_ choice: AppIconChoice) {
        Task {
            iconError = await choice.apply()
            iconChoice = AppIconChoice.current
        }
    }

    @ViewBuilder
    private func iconTile(design: AppIconChoice.Design, tint: AppIconChoice.Tint) -> some View {
        let isLight = design == .light
        RoundedRectangle(cornerRadius: 12)
            .fill(isLight ? Color.white : Color.black)
            .frame(width: 52, height: 52)
            .overlay(
                Group {
                    if let preview = design.previewImage {
                        Image(preview).resizable().scaledToFill().colorMultiply(tint.color).clipShape(RoundedRectangle(cornerRadius: 12))
                    } else {
                        RoundedRectangle(cornerRadius: 7).fill(isLight ? Color(white: 0.91) : Color(white: 0.09)).padding(9)
                            .overlay(Image(systemName: "chevron.right").font(.system(size: 14, weight: .bold)).foregroundColor(isLight ? .black : tint.color))
                    }
                }
            )
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.primary.opacity(0.25), lineWidth: 1))
    }

    private var appearancePage: some View {
        Form {
            Section(header: Text("App icon"), footer: Text(iconError ?? "The icon on the home screen. iOS shows a confirmation when it changes. iOS only allows icons that ship inside the app, so your own picture can be used for tabs and profiles (Agent profiles → Icon), not for the home screen.")) {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 12) {
                    ForEach(AppIconChoice.Design.allCases) { design in
                        let selected = iconChoice.design == design
                        Button {
                            var next = iconChoice
                            next.design = design
                            if !design.supportsTint { next.tint = .white }
                            setIcon(next)
                        } label: {
                            VStack(spacing: 4) {
                                iconTile(design: design, tint: selected ? iconChoice.tint : .white)
                                    .overlay(RoundedRectangle(cornerRadius: 14).stroke(Color.appAccent, lineWidth: selected ? 3 : 0).padding(-3))
                                Text(design.title).font(.caption2).foregroundColor(.secondary)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 6)
                if iconChoice.design.supportsTint {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 8), spacing: 10) {
                        ForEach(AppIconChoice.Tint.allCases) { tint in
                            Button {
                                var next = iconChoice
                                next.tint = tint
                                setIcon(next)
                            } label: {
                                Circle().fill(tint.color).frame(width: 26, height: 26)
                                    .overlay(Circle().stroke(Color.primary.opacity(0.3), lineWidth: 1))
                                    .overlay(Circle().stroke(Color.primary, lineWidth: iconChoice.tint == tint ? 2.5 : 0).padding(-3))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
            Section(header: Text("Accent color"), footer: Text("Buttons, selected chips and links. Open sheets update when you reopen them.")) {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 14) {
                    ForEach(AppAccent.allCases) { accent in
                        Button { accentRaw = accent.rawValue } label: {
                            VStack(spacing: 4) {
                                Circle().fill(accent.color).frame(width: 34, height: 34)
                                    .overlay(Circle().stroke(Color.primary, lineWidth: accentRaw == accent.rawValue ? 3 : 0).padding(-3))
                                Text(accent.title).font(.caption2).foregroundColor(.secondary)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 6)
                ColorPicker(selection: Binding(
                    get: { AppAccent.color(for: accentRaw) },
                    set: { accentRaw = "#" + $0.hexString }
                ), supportsOpacity: false) {
                    HStack {
                        Text("Custom color")
                        if accentRaw.hasPrefix("#") {
                            Text(accentRaw).font(.system(size: 12, design: .monospaced)).foregroundColor(.secondary)
                        }
                    }
                }
                if accentRaw.hasPrefix("#") {
                    Button("Back to Blue") { accentRaw = AppAccent.blue.rawValue }
                }
            }
            Section(
                header: Text("My color schemes"),
                footer: Text("Make your own scheme or copy one and change it. It then appears in every list above and in the profile editor.")
            ) {
                ForEach(TerminalScheme.custom) { s in
                    Button { editingScheme = s } label: {
                        HStack {
                            Text(s.name).foregroundColor(.primary)
                            Spacer()
                            HStack(spacing: 0) {
                                ForEach(1..<7, id: \.self) { Color(hex: s.ansi[$0]).frame(width: 10, height: 14) }
                            }
                            .padding(3).background(Color(hex: s.background)).cornerRadius(4)
                        }
                    }
                }
                .onDelete { offsets in
                    let list = TerminalScheme.custom
                    for i in offsets { TerminalScheme.deleteCustom(id: list[i].id) }
                    schemeRefresh += 1
                }
                Button {
                    editingScheme = TerminalScheme.newCustom(from: .fallback, name: "My scheme")
                } label: { Label("New scheme", systemImage: "plus") }
            }
            Section(
                header: Text("Terminal colors"),
                footer: Text("The scheme a tab gets by agent. A profile can override it (Agents → profile → Terminal colors). Reopen a tab or switch to it to see a change.")
            ) {
                ForEach(SchemeSlot.allCases) { slot in
                    Picker(slot.title, selection: Binding(
                        get: { _ = schemeRefresh; return TerminalSchemes.choice(for: slot) },
                        set: { TerminalSchemes.setChoice($0, for: slot); schemeRefresh += 1 }
                    )) {
                        ForEach(TerminalScheme.all) { Text($0.name).tag($0.id) }
                    }
                }
            }
            Section {
                NavigationLink {
                    SchemePreviewsView()
                } label: {
                    Label("Theme previews", systemImage: "paintpalette")
                }
            }
        }
        .navigationTitle("Appearance")
        .onAppear { iconChoice = AppIconChoice.current }
        .sheet(item: $editingScheme) { s in
            TerminalSchemeEditorView(
                scheme: s,
                onSave: { saved in
                    TerminalScheme.upsertCustom(saved)
                    schemeRefresh += 1
                    editingScheme = nil
                },
                onCancel: { editingScheme = nil }
            )
        }
    }

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
            Section(
                header: Text("New terminals"),
                footer: Text(TerminalFolder.problem(terminalFolder) ?? "Plain terminals start in this folder on the host; it is created if it does not exist. Empty = the host's home folder. Agent tabs use the folder of their profile. Terminals that already exist stay where they are.")
                    .foregroundColor(TerminalFolder.problem(terminalFolder) == nil ? .secondary : .red)
            ) {
                TextField("~/Terminals", text: $terminalFolder)
                    .font(.system(.body, design: .monospaced))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
            }
            Section(
                header: Text("Model usage line"),
                footer: Text("A thin line under the tabs: model, how full its context is, Claude tokens of the last 5 hours and a notice when a limit is reached. It asks the host every 90 s while the app is open and is not your plan's remaining quota. Tap it to refresh.")
            ) {
                Toggle("Show under the tabs", isOn: $usageStripEnabled)
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
                                        .foregroundColor(.appAccent)
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
                        if FreeTier.canAddServer(count: serversManager.profiles.count, isPro: subs.unlocked) {
                            showAddServerSheet = true
                        } else {
                            showPaywall = true
                        }
                    }) {
                        Label("Add Server...", systemImage: "plus.circle.fill")
                            .font(.subheadline.bold())
                    }
                }

        }
        .navigationTitle("Servers")
            .sheet(isPresented: $showPaywall) {
                PaywallView(reason: "The free version keeps up to \(FreeTier.maxServers) servers. Pro removes the limit.")
            }
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
                                .foregroundColor(.appAccent)
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

/// All terminal color schemes with a sample line each, on their own page so Appearance stays short.
struct SchemePreviewsView: View {
    var body: some View {
        List {
            ForEach(TerminalScheme.all) { scheme in
                VStack(alignment: .leading, spacing: 6) {
                    Text(scheme.name).font(.caption).foregroundColor(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("$ claude --resume").font(.system(size: 12, design: .monospaced))
                            .foregroundColor(Color(hex: scheme.foreground))
                        HStack(spacing: 0) {
                            ForEach(0..<16, id: \.self) { Color(hex: scheme.ansi[$0]).frame(height: 10) }
                        }
                    }
                    .padding(8)
                    .background(Color(hex: scheme.background))
                    .cornerRadius(8)
                }
            }
        }
        .navigationTitle("Theme previews")
    }
}
