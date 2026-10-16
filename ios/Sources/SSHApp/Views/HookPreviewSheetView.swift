import SwiftUI
import SshCoreBridge

/// Shows exactly what the approval hooks of a profile will write into the agent's config on the
/// host (and what is installed now) before anything is installed. Install/Remove live here only.
struct HookPreviewSheetView: View {
    let profile: SavedAgentProfile
    let handle: SshSessionHandle?
    var onChanged: () -> Void
    var onClose: () -> Void

    @State private var installed: [PlannedHook] = []
    @State private var planned: [PlannedHook] = []
    @State private var resulting: String? = nil
    @State private var configPath: String = ""
    @State private var error: String? = nil
    @State private var busy = false
    @State private var showScript = false

    private var preset: HookPreset { profile.hookPreset }
    private var usesScript: Bool { hookPresetUsesScript(preset: preset) }

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section { Text(error).font(.caption).foregroundColor(.red) }
                }
                Section("Config file on the host") {
                    Text(configPath).font(.system(size: 12, design: .monospaced))
                    if profile.agent == .claudeCode && profile.configDir == nil {
                        Label("This profile has no config directory, so hooks go into the default Claude config (~/.claude). If that is a different account (e.g. work), close this screen and set the profile's config directory first, e.g. ~/.claude-personal.", systemImage: "exclamationmark.triangle.fill")
                            .font(.caption)
                            .foregroundColor(.orange)
                    }
                }
                if profile.agent == .codex {
                    Section {
                        Label("Codex only runs hooks you have reviewed: after installing, open Codex on the host and trust them with /hooks. Changed hooks are skipped until reviewed again.", systemImage: "hand.raised.fill")
                            .font(.caption).foregroundColor(.orange)
                    }
                }
                if profile.agent == .grok {
                    Section {
                        Label("Grok hooks can only block, not approve: a phone Deny stops the tool, an Approve lets it continue into Grok's own flow. A timeout or error lets the tool run.", systemImage: "info.circle")
                            .font(.caption).foregroundColor(.secondary)
                    }
                }
                Section("Currently installed") {
                    if installed.isEmpty {
                        Text(handle == nil ? "Connect to a host to read it." : "No ssh-app hooks installed.")
                            .font(.caption).foregroundColor(.secondary)
                    }
                    ForEach(Array(installed.enumerated()), id: \.offset) { _, h in entryRow(h) }
                }
                Section("Will be installed (\(presetTitle))") {
                    if planned.isEmpty {
                        Text("Nothing: fix the preset first.").font(.caption).foregroundColor(.secondary)
                    }
                    ForEach(Array(planned.enumerated()), id: \.offset) { _, h in entryRow(h) }
                }
                if let resulting {
                    Section(header: Text("Resulting hooks section"), footer: Text("Only hooks are shown; other settings in the file are never displayed.")) {
                        Text(resulting).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                    }
                }
                if usesScript {
                    Section(
                        header: Text("Also written on the host"),
                        footer: Text("Phone approvals are verified on the host: only decisions signed by this device's key are honoured. The key is public; no secret leaves this device.")
                    ) {
                        Text("~/.ssh-app/allowed_signers: this device's public key, valid only for approval signatures")
                            .font(.system(size: 11, design: .monospaced))
                        Text("~/.ssh-app/run/: private (0700) queue folders for pending requests")
                            .font(.system(size: 11, design: .monospaced))
                    }
                    Section {
                        DisclosureGroup("Hook script (~/.ssh-app/bin/ssh-app-hook)", isExpanded: $showScript) {
                            Text(agentHookScript()).font(.system(size: 10, design: .monospaced)).textSelection(.enabled)
                        }
                    }
                }
                Section {
                    if busy {
                        ProgressView()
                    } else {
                        Button("Install these hooks") { Task { await run(install: true) } }
                            .disabled(handle == nil || planned.isEmpty)
                        Button("Remove ssh-app hooks", role: .destructive) { Task { await run(install: false) } }
                            .disabled(handle == nil || installed.isEmpty)
                    }
                }
            }
            .navigationTitle("Hooks: \(profile.name)")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
            .task { await load() }
        }
    }

    private var presetTitle: String {
        switch profile.hookKindRaw {
        case "notifyOnly": return "notify only"
        case "custom": return "custom"
        default: return "phone approval"
        }
    }

    @ViewBuilder
    private func entryRow(_ h: PlannedHook) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(h.event + (h.matcher.map { "  [\($0)]" } ?? "")).font(.subheadline.bold())
            Text(h.command).font(.system(size: 11, design: .monospaced))
        }
    }

    private func load() async {
        error = nil
        let cfg = profile.configDir
        configPath = (try? agentHooksConfigPath(agent: profile.agent, configDir: cfg)) ?? ""
        do {
            planned = try planAgentHooks(agent: profile.agent, preset: preset, hookDir: nil)
        } catch {
            planned = []
            self.error = error.localizedDescription
        }
        guard let handle else { return }
        do {
            installed = try await handle.installedAgentHooks(agent: profile.agent, configDir: cfg)
            if !planned.isEmpty {
                resulting = try await handle.previewAgentHooks(
                    agent: profile.agent, configDir: cfg, hookDir: nil, preset: preset)
            }
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func run(install: Bool) async {
        guard let handle else { return }
        busy = true
        defer { busy = false }
        do {
            if install {
                try await handle.installAgentHooksWithPreset(
                    agent: profile.agent, configDir: profile.configDir, hookDir: nil, preset: preset)
            } else {
                try await handle.uninstallAgentHooks(agent: profile.agent, configDir: profile.configDir)
            }
            onChanged()
        } catch {
            self.error = (install ? "Install failed: " : "Remove failed: ") + error.localizedDescription
        }
        await load()
    }
}
