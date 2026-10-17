import SwiftUI
import SshCoreBridge

/// What the agent of the current tab may do: the folder it runs in, the rules in effect (its
/// profile's settings and the folder's own `.claude/settings*.json`), the start mode, and
/// suggestions computed from them. Opens the permission editor with a suggestion pre-applied.
struct SessionPermissionsSheetView: View {
    let tab: TabSession
    let profile: SavedAgentProfile?
    let handle: SshSessionHandle?
    var onClose: () -> Void

    private struct EditorTarget: Identifiable {
        let id = UUID()
        let initial: PermissionSuggestion?
    }

    @State private var report: SessionPermissionsReport?
    @State private var error: String?
    @State private var loading = false
    @State private var editor: EditorTarget?

    /// The saved profile, or a stand-in for a tab that was opened by hand with an agent command.
    private var effectiveProfile: SavedAgentProfile? {
        if let profile { return profile }
        switch tab.agentType {
        case .claudeCode: return SavedAgentProfile(name: tab.title, agent: .claudeCode, binary: "claude")
        case .antigravity: return SavedAgentProfile(name: tab.title, agent: .antigravity, binary: "agy")
        default: return nil
        }
    }

    private func modeLabel(_ raw: String) -> String {
        switch raw {
        case "": return "Agent default"
        case "acceptEdits", "accept-edits": return "Accept edits"
        case "plan": return "Plan (read-only)"
        case "auto": return "Auto"
        case "bypassPermissions": return "Bypass all prompts"
        default: return raw
        }
    }

    /// For the editor: a profile without its own config folder uses the folder of the Claude
    /// that is actually running in this tab (its CLAUDE_CONFIG_DIR), when it was found.
    private var editorProfile: SavedAgentProfile? {
        guard var p = effectiveProfile else { return nil }
        if p.configDir == nil, let dir = report?.detectedConfigDir {
            p.env.removeAll { $0.hasPrefix("CLAUDE_CONFIG_DIR=") }
            p.env.append("CLAUDE_CONFIG_DIR=\(dir)")
        }
        return p
    }

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section { Text(verbatim: error).font(.caption).foregroundColor(.red) }
                }
                Section("Session") {
                    LabeledContent("Tab", value: tab.title)
                    LabeledContent("tmux", value: tab.sessionName)
                    LabeledContent("Agent", value: effectiveProfile.map { $0.name } ?? tab.agentType.rawValue)
                    if let cfg = report?.detectedConfigDir ?? effectiveProfile?.configDir {
                        LabeledContent("Config folder") {
                            Text(verbatim: cfg).font(.system(size: 12, design: .monospaced))
                                .multilineTextAlignment(.trailing)
                        }
                    }
                    if let dir = report?.directory {
                        LabeledContent("Folder") {
                            Text(verbatim: dir).font(.system(size: 12, design: .monospaced))
                                .multilineTextAlignment(.trailing)
                        }
                    }
                }
                if let p = effectiveProfile {
                    content(p)
                } else {
                    Section {
                        Text("This tab runs a plain shell or an agent without a permission model we can read, so there is nothing to review here.")
                            .font(.caption).foregroundColor(.secondary)
                    }
                }
            }
            .navigationTitle("Permissions")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) }
                ToolbarItem(placement: .primaryAction) {
                    Button { Task { await load() } } label: { Image(systemName: "arrow.clockwise") }
                        .disabled(loading || handle == nil)
                }
            }
            .task { await load() }
            .sheet(item: $editor) { target in
                if let p = editorProfile {
                    PermissionsEditorSheetView(
                        profile: p,
                        handle: handle,
                        context: [report?.projectShared, report?.projectLocal].compactMap { $0 },
                        initial: target.initial,
                        onClose: { editor = nil; Task { await load() } }
                    )
                }
            }
        }
        .interactiveDismissDisabled()
    }

    @ViewBuilder
    private func content(_ p: SavedAgentProfile) -> some View {
        Section(
            header: Text("Start mode"),
            footer: Text(p.agent == .antigravity
                ? "Antigravity takes its mode at start (Agents → profile → Permission mode); a running tab keeps the mode it started with."
                : "The mode the tab was started with (Agents → profile → Permission mode). The settings file below can also set a default mode.")
        ) {
            Text(verbatim: modeLabel(p.permissionMode))
        }
        if p.agent == .claudeCode {
            if handle == nil {
                Section { Text("Connect to a host to read the rules.").font(.caption).foregroundColor(.secondary) }
            } else if loading && report == nil {
                Section { ProgressView() }
            } else if let r = report {
                rulesSection("Profile settings", r.user, empty: "No permission rules in the profile's settings file.")
                rulesSection("Project (shared)", r.projectShared, empty: r.directory == nil ? "No folder known." : "No .claude/settings.json in the folder.")
                rulesSection("Project (local)", r.projectLocal, empty: r.directory == nil ? "No folder known." : "No .claude/settings.local.json in the folder.")
                if !r.suggestions.isEmpty {
                    Section(
                        header: Text("Suggestions"),
                        footer: Text("Tap one to open the editor with it applied. Nothing is written until you Apply there.")
                    ) {
                        ForEach(r.suggestions, id: \.id) { s in
                            Button { editor = EditorTarget(initial: s) } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Label { Text(verbatim: s.title) } icon: {
                                        Image(systemName: s.kind == .risk ? "exclamationmark.triangle.fill" : "lightbulb")
                                            .foregroundColor(s.kind == .risk ? .orange : .yellow)
                                    }
                                    Text(verbatim: s.detail).font(.caption).foregroundColor(.secondary)
                                }
                            }
                        }
                    }
                } else {
                    Section { Label("The setup already matches the suggested one.", systemImage: "checkmark.circle").font(.caption).foregroundColor(.green) }
                }
                Section {
                    Button { editor = EditorTarget(initial: nil) } label: { Label("Open permission editor", systemImage: "slider.horizontal.3") }
                }
            }
        } else if p.agent == .antigravity {
            Section("Rules") {
                Text("Antigravity keeps its command and file grants internally, so they cannot be read or edited from here. Use the start mode above; in Auto mode commands run without asking.")
                    .font(.caption).foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private func rulesSection(_ title: String, _ perms: AgentPermissions?, empty: String) -> some View {
        Section(title) {
            if let perms {
                if let m = perms.defaultMode { LabeledContent("Default mode", value: modeLabel(m)) }
                ruleGroup("Allow", perms.allow, .green)
                ruleGroup("Ask", perms.ask, .orange)
                ruleGroup("Deny", perms.deny, .red)
                if perms.allow.isEmpty && perms.ask.isEmpty && perms.deny.isEmpty && perms.defaultMode == nil {
                    Text("No rules.").font(.caption).foregroundColor(.secondary)
                }
            } else {
                Text(empty).font(.caption).foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private func ruleGroup(_ name: String, _ rules: [String], _ color: Color) -> some View {
        if !rules.isEmpty {
            DisclosureGroup {
                ForEach(rules, id: \.self) { Text(verbatim: $0).font(.system(size: 12, design: .monospaced)) }
            } label: {
                Text("\(name) · \(rules.count)").foregroundColor(color)
            }
        }
    }

    private func load() async {
        guard let handle, let p = effectiveProfile else { return }
        loading = true; defer { loading = false }
        do {
            report = try await handle.inspectSessionPermissions(agent: p.agent, configDir: p.configDir)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}
