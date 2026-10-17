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
    @State private var sessions: [AgentSessionInfo] = []
    @State private var status: String?
    @State private var busy = false

    private func lines(_ text: String) -> [String] {
        text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    private var current: HostGuard {
        HostGuard(deny: lines(deny), ask: lines(ask), allow: lines(allow),
                  sessionStartContext: sessionContext, promptContext: promptContext, stopMessage: stopMessage)
    }

    private func load(_ g: HostGuard) {
        deny = g.deny.joined(separator: "\n")
        ask = g.ask.joined(separator: "\n")
        allow = g.allow.joined(separator: "\n")
        sessionContext = g.sessionStartContext
        promptContext = g.promptContext
        stopMessage = g.stopMessage
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
