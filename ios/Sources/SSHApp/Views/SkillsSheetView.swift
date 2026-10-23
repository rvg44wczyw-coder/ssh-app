import SshCoreBridge
import SwiftUI

/// The project's skills for this tab's agent: list, create, edit, disable, delete (to a trash that
/// keeps them 30 days) and restore. Project skills are editable; user-level ones are read only.
struct SkillsSheetView: View {
    let tab: TabSession
    @ObservedObject var viewModel: TerminalSessionViewModel
    var onClose: () -> Void

    @State private var folder: String?
    @State private var skills: [SkillSummary] = []
    @State private var trash: [String] = []
    @State private var error: String?
    @State private var loading = false
    @State private var editing: EditTarget?
    @State private var creating = false
    @State private var newName = ""
    @State private var newDescription = ""
    @State private var deleting: SkillSummary?

    private struct EditTarget: Identifiable {
        let id = UUID()
        let name: String
        let userScope: Bool
        var content: String
    }

    private var agent: AgentKind? {
        switch tab.agentType {
        case .claudeCode: return .claudeCode
        case .antigravity: return .antigravity
        default: return nil
        }
    }

    /// How often the agent invoked each skill in the turns this tab has loaded (`Skill` tool calls).
    private var uses: [String: Int] {
        var n: [String: Int] = [:]
        for call in viewModel.transcriptTurns.flatMap(\.toolCalls) where call.toolName == "Skill" {
            for s in skills where call.argumentsSummary.contains(s.name) { n[s.name, default: 0] += 1 }
        }
        return n
    }

    private var project: [SkillSummary] { skills.filter { $0.scope == "project" } }
    private var user: [SkillSummary] { skills.filter { $0.scope == "user" } }
    /// What the enabled project skills put into the context before any is used: the front matter
    /// (names and descriptions); a body is read only when the agent uses the skill.
    private var activeTokens: UInt64 { project.filter(\.enabled).map(\.tokens).reduce(0, +) }

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section { Text(verbatim: error).font(.caption).foregroundColor(.red) }
                }
                if agent == nil {
                    Section { Text("This tab's agent has no skills.").foregroundColor(.secondary) }
                } else {
                    if let folder {
                        Section {
                            LabeledContent("Project") {
                                Text(verbatim: folder).font(.system(size: 12, design: .monospaced)).multilineTextAlignment(.trailing)
                            }
                            LabeledContent("Project skills", value: "\(project.filter(\.enabled).count) on · ≈\(activeTokens) tokens")
                        }
                    }
                    Section("Project skills") {
                        if project.isEmpty && !loading { Text("None yet. Tap + to create one.").foregroundColor(.secondary) }
                        ForEach(project, id: \.name) { s in row(s) }
                    }
                    if !user.isEmpty {
                        Section("User skills (read only)") {
                            ForEach(user, id: \.name) { s in
                                Button { open(s) } label: { label(s) }.buttonStyle(.plain)
                            }
                        }
                    }
                    if !trash.isEmpty {
                        Section("Trash (kept 30 days)") {
                            ForEach(trash, id: \.self) { entry in
                                HStack {
                                    Text(verbatim: displayName(entry)).font(.system(size: 14, design: .monospaced))
                                    Spacer()
                                    Button("Restore") { Task { await restore(entry) } }.buttonStyle(.glass)
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("Skills")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Done") { onClose() } }
                ToolbarItem(placement: .topBarTrailing) {
                    if loading { ProgressView() } else if agent != nil, folder != nil {
                        Button { newName = ""; newDescription = ""; creating = true } label: { Image(systemName: "plus") }
                            .accessibilityLabel("New skill")
                    }
                }
            }
            .refreshable { await load() }
            .task { await load() }
            .sheet(item: $editing) { target in
                SkillEditorView(target: target.name, readOnly: target.userScope, initial: target.content) { text in
                    try await save(target.name, text, create: false)
                }
            }
            .alert("New skill", isPresented: $creating) {
                TextField("name-with-hyphens", text: $newName).textInputAutocapitalization(.never).autocorrectionDisabled()
                TextField("What it is for", text: $newDescription)
                Button("Cancel", role: .cancel) {}
                Button("Create") { Task { await create() } }
            } message: { Text("Lowercase letters, digits and hyphens.") }
            .confirmationDialog("Delete this skill?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                                titleVisibility: .visible, presenting: deleting) { s in
                Button("Move \(s.name) to the trash", role: .destructive) { Task { await delete(s) } }
            } message: { _ in Text("It stays in the trash for 30 days and can be restored here.") }
        }
    }

    private func displayName(_ entry: String) -> String {
        entry.split(separator: "-").dropLast().joined(separator: "-")
    }

    private func label(_ s: SkillSummary) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(verbatim: s.name).font(.system(size: 15, weight: .semibold, design: .monospaced))
                    .foregroundColor(s.enabled ? .primary : .secondary)
                Spacer()
                if let used = uses[s.name] { Text("used \(used)×").font(.caption2).foregroundColor(.accentColor) }
                Text("≈\(s.tokens) tok").font(.caption2).foregroundColor(.secondary)
            }
            if !s.description.isEmpty {
                Text(verbatim: s.description).font(.caption).foregroundColor(.secondary).lineLimit(2)
            }
            if !s.enabled { Text("Disabled").font(.caption2).foregroundColor(.orange) }
        }
    }

    private func row(_ s: SkillSummary) -> some View {
        Button { open(s) } label: { label(s) }
            .buttonStyle(.plain)
            .swipeActions(edge: .trailing) {
                Button(role: .destructive) { deleting = s } label: { Label("Delete", systemImage: "trash") }
                Button { Task { await toggle(s) } } label: {
                    Label(s.enabled ? "Disable" : "Enable", systemImage: s.enabled ? "pause.circle" : "play.circle")
                }.tint(.orange)
            }
            .contextMenu {
                Button { open(s) } label: { Label("Edit", systemImage: "pencil") }
                Button { Task { await toggle(s) } } label: {
                    Label(s.enabled ? "Disable" : "Enable", systemImage: s.enabled ? "pause.circle" : "play.circle")
                }
                Button(role: .destructive) { deleting = s } label: { Label("Delete…", systemImage: "trash") }
            }
    }

    // MARK: - Host calls

    private func context() async -> (SshSessionHandle, AgentKind, String, String)? {
        guard let handle = viewModel.sessionHandle, viewModel.state == .connected, let agent else {
            error = "Connect the session first."
            return nil
        }
        guard let pin = await viewModel.helperPin() else {
            error = "The host helper is not installed: set it up in Host helper."
            return nil
        }
        if folder == nil {
            do {
                folder = try await handle.inspectSessionPermissions(agent: agent, configDir: nil).directory
            } catch { self.error = error.localizedDescription }
        }
        guard let folder else { if error == nil { error = "The project folder of this tab is unknown." }; return nil }
        return (handle, agent, folder, pin)
    }

    private func load() async {
        loading = true; defer { loading = false }
        guard let (h, a, f, pin) = await context() else { return }
        do {
            skills = try await h.listSkills(agent: a, projectDir: f, helperSha256: pin)
            trash = (try? await h.listSkillTrash(agent: a, projectDir: f, helperSha256: pin)) ?? []
            error = nil
        } catch { self.error = error.localizedDescription }
    }

    private func open(_ s: SkillSummary) {
        Task {
            guard let (h, a, f, pin) = await context() else { return }
            do {
                let doc = try await h.readSkill(agent: a, projectDir: f, name: s.name, userScope: s.readOnly, helperSha256: pin)
                editing = EditTarget(name: s.name, userScope: s.readOnly, content: doc.content)
            } catch { self.error = error.localizedDescription }
        }
    }

    private func save(_ name: String, _ text: String, create: Bool) async throws {
        guard let (h, a, f, pin) = await context() else { throw SshCoreError.InvalidInput(reason: error ?? "not connected") }
        try await h.saveSkill(agent: a, projectDir: f, name: name, content: text, create: create, helperSha256: pin)
        await load()
    }

    private func create() async {
        let name = newName.trimmingCharacters(in: .whitespaces)
        let desc = newDescription.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: "\n", with: " ")
        let text = "---\nname: \(name)\ndescription: \(desc.isEmpty ? "Describe when to use this skill" : desc)\n---\n\n# \(name)\n\n"
        do { try await save(name, text, create: true) } catch { self.error = error.localizedDescription }
    }

    private func toggle(_ s: SkillSummary) async {
        guard let (h, a, f, pin) = await context() else { return }
        do { try await h.setSkillEnabled(agent: a, projectDir: f, name: s.name, enabled: !s.enabled, helperSha256: pin) } catch { self.error = error.localizedDescription }
        await load()
    }

    private func delete(_ s: SkillSummary) async {
        guard let (h, a, f, pin) = await context() else { return }
        do { _ = try await h.deleteSkill(agent: a, projectDir: f, name: s.name, helperSha256: pin) } catch { self.error = error.localizedDescription }
        await load()
    }

    private func restore(_ entry: String) async {
        guard let (h, a, f, pin) = await context() else { return }
        do { try await h.restoreSkill(agent: a, projectDir: f, trashEntry: entry, helperSha256: pin) } catch { self.error = error.localizedDescription }
        await load()
    }
}

/// Edits one `SKILL.md` as text. The host checks the front matter on save and says what is wrong.
private struct SkillEditorView: View {
    let target: String
    let readOnly: Bool
    let initial: String
    let save: (String) async throws -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let error { Text(verbatim: error).font(.caption).foregroundColor(.red).padding(8) }
                TextEditor(text: $text)
                    .font(.system(size: 13, design: .monospaced))
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .disabled(readOnly)
                    .padding(.horizontal, 8)
            }
            .navigationTitle(target)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button(readOnly ? "Close" : "Cancel") { dismiss() } }
                if !readOnly {
                    ToolbarItem(placement: .topBarTrailing) {
                        if saving { ProgressView() } else {
                            Button("Save") {
                                saving = true
                                Task {
                                    defer { saving = false }
                                    do { try await save(text); dismiss() } catch { self.error = error.localizedDescription }
                                }
                            }
                            .disabled(text == initial)
                        }
                    }
                }
            }
            .onAppear { text = initial }
        }
    }
}
