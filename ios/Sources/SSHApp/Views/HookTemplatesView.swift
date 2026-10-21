import SwiftUI
import SshCoreBridge

/// Ready-made hook setups for one agent, grouped by category. Picking one fills the profile's
/// hook settings; nothing is written to the host until "Review & install".
struct HookTemplatesView: View {
    let agent: AgentKind
    let onPick: (HookTemplate) -> Void
    @Environment(\.dismiss) private var dismiss

    private let order = ["Approvals", "Notifications", "Guards", "Logging", "Workflow"]

    var body: some View {
        let all = hookTemplatesFor(agent: agent)
        List {
            ForEach(order, id: \.self) { category in
                let items = all.filter { $0.category == category }
                if !items.isEmpty {
                    Section(category) {
                        ForEach(items, id: \.id) { t in
                            Button {
                                onPick(t)
                                dismiss()
                            } label: {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(t.title).font(.body.weight(.semibold)).foregroundColor(.primary)
                                    Text(t.description).font(.caption).foregroundColor(.secondary)
                                    if !t.caution.isEmpty {
                                        Label(t.caution, systemImage: "exclamationmark.triangle")
                                            .font(.caption2).foregroundColor(.orange)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Hook templates")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
    }
}

/// Rules and texts kept on the host in ~/.ssh-app/guard.json and enforced by the hook script:
/// block, always ask, allow, add context, and ask the agent to continue when it stops.
struct HostGuardView: View {
    let handle: SshSessionHandle?
    @Environment(\.dismiss) private var dismiss

    @State private var loaded = false
    @State private var deny = ""
    @State private var ask = ""
    @State private var allow = ""
    @State private var sessionContext = ""
    @State private var promptContext = ""
    @State private var stopMessage = ""
    /// 0 = default (120 s).
    @State private var approvalWait = 0
    /// Auto-approval by category; edited on its own screen, written together with the rest.
    @State private var auto = AutoPolicy(presetId: nil, levels: [], agentOverrides: [], projects: [],
                                         inspectAnywhere: false, trustMaxSecs: 0)
    @State private var sessions: [AgentSessionInfo] = []
    @State private var status: String?
    @State private var busy = false

    private func lines(_ text: String) -> [String] {
        text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    private var current: HostGuard {
        HostGuard(deny: lines(deny), ask: lines(ask), allow: lines(allow),
                  sessionStartContext: sessionContext, promptContext: promptContext, stopMessage: stopMessage,
                  approvalWaitSecs: UInt32(approvalWait), auto: auto)
    }

    private func load(_ g: HostGuard) {
        deny = g.deny.joined(separator: "\n")
        ask = g.ask.joined(separator: "\n")
        allow = g.allow.joined(separator: "\n")
        sessionContext = g.sessionStartContext
        promptContext = g.promptContext
        stopMessage = g.stopMessage
        approvalWait = Int(g.approvalWaitSecs)
        auto = g.auto
    }

    private func merge(_ p: HostGuard) {
        func add(_ field: inout String, _ rules: [String]) {
            var have = lines(field)
            for r in rules where !have.contains(r) { have.append(r) }
            field = have.joined(separator: "\n")
        }
        add(&deny, p.deny); add(&ask, p.ask); add(&allow, p.allow)
        if !p.sessionStartContext.isEmpty { sessionContext = p.sessionStartContext }
        if !p.promptContext.isEmpty { promptContext = p.promptContext }
        if !p.stopMessage.isEmpty { stopMessage = p.stopMessage }
    }

    private func ruleEditor(_ title: String, _ text: Binding<String>, _ footer: String) -> some View {
        Section(header: Text(title), footer: Text(footer)) {
            TextEditor(text: text)
                .font(.system(size: 13, design: .monospaced))
                .frame(minHeight: 80)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                if handle == nil {
                    Section { Text("Connect to a host to read and change its guard.").foregroundColor(.secondary) }
                }
                Section(
                    header: Text("Add a ready-made set"),
                    footer: Text("Sets are merged into the lists below; nothing is written until you press Apply.")
                ) {
                    Menu {
                        ForEach(hostGuardPresets(), id: \.id) { p in
                            Button { merge(p.guard) } label: { Text(p.title) }
                        }
                    } label: { Label("Presets", systemImage: "square.stack.3d.up") }
                }
                Section(
                    footer: Text("Allows whole categories of commands (cargo, git, python…) inside your projects, with presets. The rules it generates are kept apart from the lists below.")
                ) {
                    NavigationLink {
                        AutoApprovalView(policy: $auto, handle: handle, sessions: sessions)
                    } label: {
                        Label("Auto-approval by category", systemImage: "checkmark.shield")
                    }
                }
                ruleEditor("Block (deny)", $deny, "One rule per line, e.g. Bash(rm:*) or Read(~/.ssh/**). Refused on the host without asking anyone. Always wins.")
                ruleEditor("Always ask", $ask, "Handed to you even if the agent's mode would not ask (Claude, Antigravity).")
                ruleEditor("Allow without asking", $allow, "Runs without a prompt (Claude, Antigravity, Codex). Use narrow rules.")
                Section(
                    header: Text("Context for Claude"),
                    footer: Text("Text you write here is added to what the model reads, with your authority.")
                ) {
                    TextField("When a session starts", text: $sessionContext, axis: .vertical)
                    TextField("With every prompt", text: $promptContext, axis: .vertical)
                }
                Section(
                    header: Text("When Claude stops"),
                    footer: Text("Asks it to continue once per stop with this message. Use something that ends.")
                ) {
                    TextField("Message (empty = off)", text: $stopMessage, axis: .vertical)
                }
                Section(
                    header: Text("Waiting for the phone"),
                    footer: Text("How long a tool call is held for your answer from the phone. After that the agent asks in its own terminal. The question does not appear in the terminal while the hook waits.")
                ) {
                    Picker("Wait", selection: $approvalWait) {
                        Text("Default (2 min)").tag(0)
                        ForEach([10, 15, 30, 60, 90], id: \.self) { Text("\($0) s").tag($0) }
                    }
                }
                let warnings = hostGuardWarnings(guard: current)
                if !warnings.isEmpty {
                    Section("Read twice") {
                        ForEach(warnings, id: \.self) { Label($0, systemImage: "exclamationmark.triangle").font(.caption).foregroundColor(.orange) }
                    }
                }
                if !sessions.isEmpty {
                    Section(
                        header: Text("Registered sessions"),
                        footer: Text("From the \"Register sessions for the app\" template: which tmux session each agent session runs in.")
                    ) {
                        ForEach(sessions, id: \.session) { s in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(verbatim: "\(s.agent) · \(s.tmux.isEmpty ? "no tmux" : s.tmux)").font(.subheadline)
                                Text(verbatim: s.cwd).font(.caption).foregroundColor(.secondary)
                            }
                        }
                    }
                }
                if let status {
                    Section { Text(status).font(.caption).foregroundColor(status.hasPrefix("Saved") ? .green : .red) }
                }
                Section {
                    Button {
                        Task { await apply() }
                    } label: {
                        HStack { Text("Apply to the host"); if busy { Spacer(); ProgressView() } }
                    }
                    .disabled(handle == nil || busy || !loaded)
                }
            }
            .navigationTitle("Host guard")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } } }
            .task { await read() }
        }
    }

    private func read() async {
        guard let handle else { return }
        do {
            load(try await handle.readHostGuard())
            loaded = true
            sessions = (try? await handle.listAgentSessions()) ?? []
        } catch {
            status = "Could not read the guard: \(error.localizedDescription)"
        }
    }

    private func apply() async {
        guard let handle else { return }
        busy = true
        defer { busy = false }
        do {
            try await handle.applyHostGuard(guard: current)
            load(try await handle.readHostGuard())
            status = "Saved to ~/.ssh-app/guard.json"
        } catch {
            status = error.localizedDescription
        }
    }
}


private extension AutoLevel {
    var title: String {
        switch self {
        case .ask: return "Ask"
        case .auto: return "Auto"
        case .block: return "Block"
        }
    }
}

private extension AutoRisk {
    var title: String {
        switch self {
        case .readOnly: return "read-only"
        case .localChange: return "changes files"
        case .runsProjectCode: return "runs project code"
        case .network: return "network"
        }
    }
    var color: Color {
        switch self {
        case .readOnly: return .green
        case .localChange: return .yellow
        case .runsProjectCode: return .orange
        case .network: return .red
        }
    }
}

private func levelOf(_ list: [AutoCategoryLevel], _ category: String) -> AutoLevel? {
    list.first { $0.category == category }?.level
}

private func setLevel(_ list: inout [AutoCategoryLevel], _ category: String, _ level: AutoLevel?) {
    list.removeAll { $0.category == category }
    if let level { list.append(AutoCategoryLevel(category: category, level: level)) }
}

/// Auto-approval by command category (feature 50): which categories the host lets an agent run
/// without asking, inside which projects, per agent, and for how long a phone approval may extend.
struct AutoApprovalView: View {
    @Binding var policy: AutoPolicy
    let handle: SshSessionHandle?
    let sessions: [AgentSessionInfo]

    @AppStorage("trust_never_offer") private var neverOfferTrust = false
    @AppStorage("trust_default_secs") private var defaultTrustSecs = 3600
    @State private var newRoot = ""
    @State private var trusts: [AutoTrust] = []
    @State private var log: [AutoLogEntry] = []
    @State private var note: String?

    private let categories = autoCategories()
    private let agents = ["claude", "antigravity", "codex", "qwen", "crush", "opencode"]

    var body: some View {
        Form {
            presetSection
            Section(
                header: Text("Categories"),
                footer: Text("Auto applies inside the projects below. Block applies everywhere. Nothing destructive, no secrets, no sudo and no edits of agent settings are ever automatic.")
            ) {
                ForEach(categories, id: \.id) { c in
                    categoryRow(c)
                }
            }
            projectsSection
            Section(
                footer: Text("Lets the read-only category (Read, Glob, Grep, ls, git status…) run anywhere, still behind the always-blocked list.")
            ) {
                Toggle("Read-only inspection anywhere", isOn: $policy.inspectAnywhere)
            }
            Section("Agents") {
                ForEach(agents, id: \.self) { agent in
                    NavigationLink(agent.capitalized) { AutoAgentView(policy: $policy, agent: agent) }
                }
            }
            trustSection
            let warnings = autoPolicyWarnings(policy: policy)
            if !warnings.isEmpty {
                Section("Read twice") {
                    ForEach(warnings, id: \.self) {
                        Label($0, systemImage: "exclamationmark.triangle").font(.caption).foregroundColor(.orange)
                    }
                }
            }
            if let note { Section { Text(note).font(.caption).foregroundColor(.red) } }
            Section(
                header: Text("Recent automatic decisions"),
                footer: Text("Kept on the host in ~/.ssh-app/run/auto.log; shows what was allowed or refused without asking you.")
            ) {
                if log.isEmpty { Text("Nothing yet").foregroundColor(.secondary) }
                ForEach(Array(log.reversed().enumerated()), id: \.offset) { _, e in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: "\(e.decision) · \(e.agent) · \(e.rule)").font(.caption.weight(.semibold))
                        Text(verbatim: e.summary).font(.system(size: 11, design: .monospaced)).foregroundColor(.secondary).lineLimit(2)
                    }
                }
            }
        }
        .navigationTitle("Auto-approval")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .task { await refreshHost() }
        .safeAreaInset(edge: .bottom) {
            Text("Changes are written with \"Apply to the host\" on the previous screen.")
                .font(.caption2).foregroundColor(.secondary).padding(6)
        }
    }

    // MARK: presets and categories

    private var presetSection: some View {
        Section(
            header: Text("Preset"),
            footer: Text(autoPresets().first { $0.id == policy.presetId }?.description
                         ?? "Custom: the levels below were changed by hand.")
        ) {
            Picker("Preset", selection: Binding(
                get: { policy.presetId ?? "custom" },
                set: { id in
                    guard id != "custom" else { policy.presetId = nil; return }
                    do { policy = try autoPolicyWithPreset(current: policy, presetId: id); note = nil }
                    catch { note = error.localizedDescription }
                })) {
                ForEach(autoPresets(), id: \.id) { Text($0.title).tag($0.id) }
                Text("Custom").tag("custom")
            }
        }
    }

    private func categoryRow(_ c: AutoCategory) -> some View {
        DisclosureGroup {
            Text(c.description).font(.caption).foregroundColor(.secondary)
            ForEach(c.commands, id: \.rule) { k in
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: k.rule).font(.system(size: 11, design: .monospaced))
                    if !k.note.isEmpty { Text(k.note).font(.caption2).foregroundColor(k.risk.color) }
                }
            }
        } label: {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(c.title)
                    Text(c.risk.title).font(.caption2).foregroundColor(c.risk.color)
                }
                Spacer()
                Picker("", selection: Binding(
                    get: { levelOf(policy.levels, c.id) ?? .ask },
                    set: { setLevel(&policy.levels, c.id, $0 == .ask ? nil : $0); policy.presetId = nil })) {
                    ForEach([AutoLevel.ask, .auto, .block], id: \.self) { Text($0.title).tag($0) }
                }
                .pickerStyle(.menu)
                .labelsHidden()
            }
        }
    }

    // MARK: projects

    private var projectsSection: some View {
        Section(
            header: Text("Projects"),
            footer: Text("Auto only works for a session started inside one of these folders, and only for commands that stay inside it. Leaving the folder always asks you.")
        ) {
            ForEach(policy.projects.indices, id: \.self) { i in
                NavigationLink {
                    AutoProjectView(project: $policy.projects[i], policy: policy)
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: policy.projects[i].label)
                        Text(verbatim: policy.projects[i].root).font(.caption).foregroundColor(.secondary)
                    }
                }
            }
            .onDelete { policy.projects.remove(atOffsets: $0) }
            let known = Set(policy.projects.map(\.root))
            let suggestions = Array(Set(sessions.map(\.cwd)).subtracting(known)).filter { $0.hasPrefix("/") }.sorted()
            if !suggestions.isEmpty {
                Menu {
                    ForEach(suggestions, id: \.self) { dir in Button(dir) { addProject(dir) } }
                } label: { Label("Add a folder of a running session", systemImage: "folder.badge.plus") }
            }
            HStack {
                TextField("/Users/me/project", text: $newRoot)
                    .font(.system(size: 13, design: .monospaced))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                Button("Add") { addProject(newRoot); newRoot = "" }
                    .disabled(!newRoot.hasPrefix("/"))
            }
        }
    }

    private func addProject(_ root: String) {
        var p = policy
        p.projects.append(AutoProject(root: root, label: "", overrides: [], allowFileEdits: false))
        do {
            policy = try autoPolicyValidate(policy: p)
            note = nil
        } catch { note = error.localizedDescription }
    }

    // MARK: trust

    private var trustSection: some View {
        Group {
            Section(
                header: Text("Temporary trust"),
                footer: Text("On an approval card: \"Approve, and allow … for …\". The phone signs it; it covers the same agent session and project only, and ends with the session.")
            ) {
                Toggle("Offer it on approval cards", isOn: Binding(get: { !neverOfferTrust }, set: { neverOfferTrust = !$0 }))
                Picker("Longest the host honours", selection: $policy.trustMaxSecs) {
                    Text("24 h (default)").tag(UInt32(0))
                    Text("1 h").tag(UInt32(3600))
                    Text("4 h").tag(UInt32(4 * 3600))
                    Text("12 h").tag(UInt32(12 * 3600))
                }
            }
            Section("Active trusts") {
                if trusts.isEmpty { Text("None").foregroundColor(.secondary) }
                ForEach(trusts, id: \.id) { t in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "\(t.agent) · \(t.root)").font(.caption.weight(.semibold))
                            Text(verbatim: t.rules.joined(separator: ", ")).font(.system(size: 10, design: .monospaced)).foregroundColor(.secondary).lineLimit(2)
                            Text(Date(timeIntervalSince1970: TimeInterval(t.expiresSec)), style: .relative)
                                .font(.caption2).foregroundColor(.secondary)
                        }
                        Spacer()
                        Button("Revoke", role: .destructive) { Task { try? await handle?.revokeTrust(id: t.id); await refreshHost() } }
                            .buttonStyle(.borderless)
                    }
                }
                if !trusts.isEmpty {
                    Button("Revoke all", role: .destructive) { Task { try? await handle?.revokeAllTrusts(); await refreshHost() } }
                }
            }
        }
    }

    private func refreshHost() async {
        guard let handle else { return }
        trusts = (try? await handle.listTrusts()) ?? []
        log = (try? await handle.readAutoLog()) ?? []
    }
}

/// Levels of one agent that differ from the default.
struct AutoAgentView: View {
    @Binding var policy: AutoPolicy
    let agent: String

    private var index: Int? { policy.agentOverrides.firstIndex { $0.agent == agent } }

    var body: some View {
        Form {
            Section(footer: Text("\"Default\" follows the levels of the main screen.")) {
                ForEach(autoCategories(), id: \.id) { c in
                    Picker(c.title, selection: Binding(
                        get: { index.flatMap { levelOf(policy.agentOverrides[$0].levels, c.id) } },
                        set: { set(c.id, $0) })) {
                        Text("Default").tag(AutoLevel?.none)
                        ForEach([AutoLevel.ask, .auto, .block], id: \.self) { Text($0.title).tag(AutoLevel?.some($0)) }
                    }
                }
            }
        }
        .navigationTitle(agent.capitalized)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
    }

    private func set(_ category: String, _ level: AutoLevel?) {
        var i = index
        if i == nil {
            policy.agentOverrides.append(AutoAgentOverride(agent: agent, levels: []))
            i = policy.agentOverrides.count - 1
        }
        guard let i else { return }
        setLevel(&policy.agentOverrides[i].levels, category, level)
        if policy.agentOverrides[i].levels.isEmpty { policy.agentOverrides.remove(at: i) }
        policy.presetId = nil
    }
}

/// One project: its name, file edits, and categories that differ from the default. A project can
/// only choose Ask or Auto; it cannot lift a Block.
struct AutoProjectView: View {
    @Binding var project: AutoProject
    let policy: AutoPolicy

    var body: some View {
        Form {
            Section("Project") {
                TextField("Name", text: $project.label)
                Text(verbatim: project.root).font(.system(size: 12, design: .monospaced)).foregroundColor(.secondary)
            }
            Section(
                footer: Text("Edit/Write inside this folder run without asking, except in .git, .claude and other agent settings. Needs \"Files\" on Auto. Outside the folder it always asks.")
            ) {
                Toggle("Allow file edits inside the project", isOn: $project.allowFileEdits)
            }
            Section(header: Text("Categories here"), footer: Text("\"Default\" follows the main screen. Block is set there, not per project.")) {
                ForEach(autoCategories(), id: \.id) { c in
                    let blocked = levelOf(policy.levels, c.id) == .block
                    Picker(c.title, selection: Binding(
                        get: { levelOf(project.overrides, c.id) },
                        set: { setLevel(&project.overrides, c.id, $0) })) {
                        Text("Default").tag(AutoLevel?.none)
                        Text("Ask").tag(AutoLevel?.some(.ask))
                        Text("Auto").tag(AutoLevel?.some(.auto))
                    }
                    .disabled(blocked)
                }
            }
        }
        .navigationTitle(project.label)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
    }
}
