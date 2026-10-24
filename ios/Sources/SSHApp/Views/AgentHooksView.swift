import SwiftUI
import SshCoreBridge

/// The hooks of one agent profile (feature 57): what each hook event is for, and per event whether
/// the ssh-app hook, the user's own command, or nothing handles it. The ssh-app hook can also get
/// an extra command that runs before or after it.
struct AgentHooksView: View {
    @Binding var profile: SavedAgentProfile
    let error: String?

    private enum Choice: Hashable { case off, ours, own }

    private var isCustom: Bool { profile.hookKindRaw == "custom" }
    private var isOff: Bool { profile.hookKindRaw == "off" }
    private var supportsOwn: Bool { profile.agent != .crush && profile.agent != .openCode }

    private var oursEvents: [String] {
        isCustom || isOff ? [] : profile.approvalEventChoices + profile.extraApprovalEvents
    }

    private var activeOurs: Set<String> {
        let defaults = profile.approvalEventChoices
        return Set(profile.approvalEvents.isEmpty ? defaults : profile.approvalEvents).intersection(oursEvents)
    }

    private func looksLikeCredential(_ command: String) -> Bool {
        let c = command.lowercased()
        return ["bearer ", "token=", "password", "passwd", "secret", "apikey", "api_key", "sk-"].contains { c.contains($0) }
    }

    private func choice(_ event: String) -> Choice {
        if activeOurs.contains(event) { return .ours }
        if profile.hookOwn.contains(where: { $0.event == event }) { return .own }
        return .off
    }

    private func setChoice(_ event: String, _ new: Choice) {
        var ours = activeOurs
        ours.remove(event)
        profile.hookOwn.removeAll { $0.event == event }
        profile.hookExtras.removeAll { $0.event == event && new != .ours }
        switch new {
        case .ours: ours.insert(event)
        case .own: profile.hookOwn.append(SavedHookOwn(event: event))
        case .off: break
        }
        storeOurs(ours)
    }

    /// Stores the chosen ssh-app events; "all defaults" is kept as empty, nothing at all is the Off mode.
    private func storeOurs(_ ours: Set<String>) {
        if ours.isEmpty, !isOff {
            profile.hookKindRaw = "off"
            profile.approvalEvents = []
            profile.hookExtras = []
        } else if ours == Set(profile.approvalEventChoices) {
            profile.approvalEvents = []
        } else {
            profile.approvalEvents = (profile.approvalEventChoices + profile.extraApprovalEvents).filter(ours.contains)
        }
    }

    private func extraIndex(_ event: String) -> Int? {
        profile.hookExtras.firstIndex { $0.event == event }
    }

    private func ownIndex(_ event: String) -> Int? {
        profile.hookOwn.firstIndex { $0.event == event }
    }

    var body: some View {
        Form {
            Section {
                Picker("Mode", selection: $profile.hookKindRaw) {
                    Text("Phone approval").tag("phoneApproval")
                    Text("Notify only").tag("notifyOnly")
                    Text("Custom command").tag("custom")
                    Text("Off").tag("off")
                }
                NavigationLink {
                    HookTemplatesView(agent: profile.agent) { t in profile.apply(template: t.preset) }
                } label: {
                    Label("Templates (\(hookTemplatesFor(agent: profile.agent).count))", systemImage: "square.grid.3x3.square")
                }
            } footer: {
                Text(modeSummary)
            }

            if isCustom {
                customSection
            } else {
                Section("Events") {
                    ForEach(hookEventsFor(agent: profile.agent), id: \.self) { eventRow($0) }
                }
                if profile.hookKindRaw == "phoneApproval" {
                    Section {
                        TextField("Tools, e.g. Bash|Edit|Write", text: $profile.approvalMatcher)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                    } header: {
                        Text("Tools sent to the phone")
                    } footer: {
                        Text("Empty = all tools.")
                    }
                }
            }
            if let error {
                Section { Text(error).font(.caption).foregroundColor(.red) }
            }
        }
        .navigationTitle("Hooks")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .onChange(of: profile.hookKindRaw) { _, kind in
            // Switching the mode keeps what still makes sense: events the mode does not offer are dropped.
            let valid = Set(profile.approvalEventChoices + profile.extraApprovalEvents)
            profile.approvalEvents.removeAll { !valid.contains($0) }
            if kind == "custom" || kind == "off" { profile.hookExtras = [] }
            if kind == "custom" { profile.hookOwn = [] }
        }
    }

    private var modeSummary: LocalizedStringKey {
        switch profile.hookKindRaw {
        case "notifyOnly": return "The agent tells the phone when it needs you or is done. Nothing is asked."
        case "custom": return "Your own command runs for the events you pick."
        case "off": return "ssh-app installs no hook of its own. You can still run your own command per event."
        default: return "Permission requests reach the phone, where you allow or deny them."
        }
    }

    @ViewBuilder
    private func eventRow(_ event: String) -> some View {
        let info = hookEventInfo(agent: profile.agent, event: event)
        let current = choice(event)
        DisclosureGroup {
            Text(info.summary).font(.footnote).foregroundColor(.secondary)
            Picker("Handled by", selection: Binding(get: { current }, set: { setChoice(event, $0) })) {
                Text("Nobody").tag(Choice.off)
                if oursEvents.contains(event) { Text("ssh-app").tag(Choice.ours) }
                if supportsOwn { Text("My command").tag(Choice.own) }
            }
            .pickerStyle(.segmented)
            if current == .ours { extraEditor(event, info: info) }
            if current == .own, let i = ownIndex(event) { ownEditor(i, info: info) }
        } label: {
            HStack {
                Text(info.title)
                Spacer()
                Text(current == .ours ? "ssh-app" : current == .own ? "Mine" : "—")
                    .font(.footnote).foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private func extraEditor(_ event: String, info: HookEventInfo) -> some View {
        if hookPresetUsesScript(preset: profile.hookPreset) {
            Toggle("Extra command", isOn: Binding(
                get: { extraIndex(event) != nil },
                set: { on in
                    profile.hookExtras.removeAll { $0.event == event }
                    if on { profile.hookExtras.append(SavedHookExtra(event: event)) }
                }))
            if let i = extraIndex(event) {
                commandField("Command", text: $profile.hookExtras[i].command)
                Picker("Runs", selection: $profile.hookExtras[i].runsAfter) {
                    Text("Before the ssh-app hook").tag(false)
                    Text("After it").tag(true)
                }
                if info.canBlock && !profile.hookExtras[i].runsAfter {
                    Toggle("Stop the hook if it fails", isOn: $profile.hookExtras[i].stopOnFailure)
                }
                Stepper("Time limit: \(profile.hookExtras[i].timeoutSecs) s",
                        value: $profile.hookExtras[i].timeoutSecs, in: 1...60)
                warnings(profile.hookExtras[i].command)
            }
        }
    }

    @ViewBuilder
    private func ownEditor(_ i: Int, info: HookEventInfo) -> some View {
        commandField("Command", text: $profile.hookOwn[i].command)
        if info.takesMatcher {
            TextField("Tool matcher (empty = default)", text: $profile.hookOwn[i].matcher)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
        }
        warnings(profile.hookOwn[i].command)
    }

    private func commandField(_ title: LocalizedStringKey, text: Binding<String>) -> some View {
        TextField(title, text: text, axis: .vertical)
            .font(.system(size: 13, design: .monospaced))
            .autocorrectionDisabled()
            #if os(iOS)
            .textInputAutocapitalization(.never)
            #endif
    }

    @ViewBuilder
    private func warnings(_ command: String) -> some View {
        if looksLikeCredential(command) {
            Text("This command seems to contain a credential. It is stored unencrypted on this phone and written into the agent's config on the host. Read it from the host's environment instead.")
                .font(.caption).foregroundColor(.orange)
        }
        ForEach(lintCustomHookCommand(command: command), id: \.self) { w in
            Label(w, systemImage: "exclamationmark.triangle").font(.caption).foregroundColor(.orange)
        }
    }

    @ViewBuilder
    private var customSection: some View {
        Section("Command") {
            commandField("Command run by the agent", text: $profile.customHookCommand)
            warnings(profile.customHookCommand)
        }
        Section("Events") {
            ForEach(hookEventsFor(agent: profile.agent), id: \.self) { ev in
                let info = hookEventInfo(agent: profile.agent, event: ev)
                Toggle(isOn: Binding(
                    get: { profile.customHookEvents.contains(ev) },
                    set: { on in
                        profile.customHookEvents.removeAll { $0 == ev }
                        if on { profile.customHookEvents.append(ev) }
                    })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(info.title)
                        Text(info.summary).font(.caption).foregroundColor(.secondary)
                    }
                }
            }
            TextField("Tool matcher (empty = default)", text: $profile.customHookMatcher)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
        }
    }
}
