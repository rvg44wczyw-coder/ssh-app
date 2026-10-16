import SwiftUI
import SshCoreBridge

/// Edits the agent's own permission rules (allow / ask / deny / default mode) in the profile's
/// config directory on the host. Nothing is written until Apply, after a preview of the result.
struct PermissionsEditorSheetView: View {
    let profile: SavedAgentProfile
    let handle: SshSessionHandle?
    var onClose: () -> Void

    @State private var allow: [String] = []
    @State private var ask: [String] = []
    @State private var deny: [String] = []
    @State private var mode: String = ""          // "" = leave the agent's default
    @State private var loaded = false
    @State private var error: String? = nil
    @State private var busy = false
    @State private var preview: String? = nil
    @State private var warnings: [String] = []
    @State private var confirmBypass = false
    @State private var pendingMode = ""
    @State private var applied = false

    private static let modes = ["", "default", "acceptEdits", "plan", "bypassPermissions"]

    private var configPath: String {
        (try? agentHooksConfigPath(agent: profile.agent, configDir: profile.configDir)) ?? "?"
    }

    private var modeOptions: [String] {
        Self.modes.contains(mode) ? Self.modes : Self.modes + [mode]
    }

    private var current: AgentPermissions {
        AgentPermissions(allow: allow, ask: ask, deny: deny, defaultMode: mode.isEmpty ? nil : mode)
    }

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section { Text(verbatim: error).font(.caption).foregroundColor(.red) }
                }
                Section("Config file on the host") {
                    Text(verbatim: configPath).font(.system(size: 12, design: .monospaced))
                    if profile.configDir == nil {
                        Label("This profile has no config directory, so these rules go into the default Claude config (~/.claude). If that is a different account (e.g. work), close this screen and set the profile's config directory first.", systemImage: "exclamationmark.triangle.fill")
                            .font(.caption).foregroundColor(.orange)
                    }
                }
                if handle == nil {
                    Section { Text("Connect to a host to edit permissions.").font(.caption).foregroundColor(.secondary) }
                } else if loaded {
                    ruleSection("Allow (no prompt)", rules: $allow)
                    ruleSection("Ask (always prompt)", rules: $ask)
                    ruleSection("Deny (never)", rules: $deny)
                    Section("Default mode") {
                        Picker("Mode", selection: Binding(get: { mode }, set: { new in
                            if new == "bypassPermissions" { pendingMode = new; confirmBypass = true } else { mode = new }
                            preview = nil
                        })) {
                            ForEach(modeOptions, id: \.self) { m in
                                Text(verbatim: m.isEmpty ? "Agent default" : m).tag(m)
                            }
                        }
                    }
                    Section(header: Text("Templates"), footer: Text("A template adds its rules to the lists above; nothing is written until Apply.")) {
                        ForEach(permissionRuleTemplates(), id: \.id) { t in
                            Button { add(t) } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(t.title)
                                    Text(t.description).font(.caption).foregroundColor(.secondary)
                                }
                            }
                        }
                    }
                    if let preview {
                        if !warnings.isEmpty {
                            Section("Warnings") {
                                ForEach(warnings, id: \.self) { w in
                                    Label { Text(verbatim: w) } icon: { Image(systemName: "exclamationmark.triangle") }
                                        .font(.caption).foregroundColor(.orange)
                                }
                            }
                        }
                        Section(header: Text("Resulting permissions"), footer: Text("Only this section is shown; other settings in the file are never displayed.")) {
                            Text(verbatim: preview).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                        }
                    }
                    Section {
                        if busy {
                            ProgressView()
                        } else {
                            Button("Preview") { Task { await runPreview() } }
                            Button(applied ? "Applied" : "Apply to host") { Task { await apply() } }
                                .disabled(preview == nil || applied)
                        }
                    }
                }
            }
            .navigationTitle("Permissions")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
            .confirmationDialog("Bypass all permission prompts?", isPresented: $confirmBypass, titleVisibility: .visible) {
                Button("Use bypassPermissions", role: .destructive) { mode = pendingMode; preview = nil }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The agent stops asking, so nothing is sent to your phone and every tool runs unattended.")
            }
            .task { await load() }
        }
    }

    @ViewBuilder
    private func ruleSection(_ title: String, rules: Binding<[String]>) -> some View {
        Section(title) {
            ForEach(Array(rules.wrappedValue.enumerated()), id: \.offset) { i, r in
                Text(verbatim: r).font(.system(size: 13, design: .monospaced))
                    .swipeActions {
                        Button(role: .destructive) { rules.wrappedValue.remove(at: i); preview = nil } label: { Label("Delete", systemImage: "trash") }
                    }
            }
            AddRuleRow { rule in
                let r = rule.trimmingCharacters(in: .whitespaces)
                if !r.isEmpty, !rules.wrappedValue.contains(r) { rules.wrappedValue.append(r); preview = nil }
            }
        }
    }

    private func add(_ t: PermissionTemplate) {
        for (rules, new) in [($allow, t.allow), ($ask, t.ask), ($deny, t.deny)] {
            for r in new where !rules.wrappedValue.contains(r) { rules.wrappedValue.append(r) }
        }
        preview = nil
    }

    private func load() async {
        guard let handle else { return }
        do {
            let p = try await handle.readAgentPermissions(agent: profile.agent, configDir: profile.configDir)
            allow = p.allow; ask = p.ask; deny = p.deny; mode = p.defaultMode ?? ""
            loaded = true
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func runPreview() async {
        guard let handle else { return }
        busy = true; defer { busy = false }
        do {
            let p = try validateAgentPermissions(agent: profile.agent, permissions: current)
            preview = try await handle.previewAgentPermissions(agent: profile.agent, configDir: profile.configDir, permissions: p)
            warnings = permissionWarnings(permissions: p)
            applied = false
            error = nil
        } catch {
            preview = nil
            self.error = error.localizedDescription
        }
    }

    private func apply() async {
        guard let handle else { return }
        busy = true; defer { busy = false }
        do {
            let p = try validateAgentPermissions(agent: profile.agent, permissions: current)
            try await handle.applyAgentPermissions(agent: profile.agent, configDir: profile.configDir, permissions: p)
            allow = p.allow; ask = p.ask; deny = p.deny
            applied = true
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private struct AddRuleRow: View {
    var onAdd: (String) -> Void
    @State private var text = ""

    var body: some View {
        HStack {
            TextField("Rule, e.g. Bash(git push:*)", text: $text)
                .font(.system(size: 13, design: .monospaced))
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
                .onSubmit(commit)
            Button(action: commit) { Image(systemName: "plus.circle.fill") }
                .buttonStyle(.borderless)
                .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
        }
    }

    private func commit() {
        onAdd(text)
        text = ""
    }
}
