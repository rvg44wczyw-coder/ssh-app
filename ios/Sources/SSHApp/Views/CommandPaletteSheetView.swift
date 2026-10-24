import SwiftUI

/// Slash commands of the CLI agents, so they do not have to be typed on a phone keyboard.
/// Commands are inserted at the prompt (to add arguments) or sent straight away.
struct AgentSlashCommand: Identifiable, Hashable {
    let name: String
    let detail: String
    var id: String { name }
}

enum PaletteAgent: String, CaseIterable, Identifiable {
    case claude = "Claude Code"
    case codex = "Codex"
    case antigravity = "Antigravity"
    case grok = "Grok"
    case ollama = "Ollama"
    var id: String { rawValue }

    init(tab: AgentType) {
        switch tab {
        case .claudeCode: self = .claude
        case .antigravity: self = .antigravity
        case .localLlm: self = .ollama
        case .customShell, .dockerContainer: self = .claude
        }
    }

    /// Only commands known to exist; an agent without a catalogue still gets the free-form field.
    var commands: [AgentSlashCommand] {
        switch self {
        case .claude:
            return [
                ("/model", "Choose the model"), ("/config", "Open settings"), ("/status", "Version, account, connectivity"),
                ("/usage", "Plan usage and limits"), ("/cost", "Token usage of this session"), ("/context", "What fills the context window"),
                ("/compact", "Summarise the conversation to free context"), ("/clear", "Start a fresh conversation"),
                ("/resume", "Resume an earlier conversation"), ("/permissions", "Allow / ask / deny rules"), ("/hooks", "Configured hooks"),
                ("/mcp", "MCP servers"), ("/agents", "Subagents"), ("/memory", "Edit CLAUDE.md memory"), ("/init", "Create CLAUDE.md"),
                ("/review", "Review a pull request"), ("/doctor", "Check the installation"), ("/login", "Sign in"), ("/logout", "Sign out"),
                ("/help", "List all commands"),
            ].map(AgentSlashCommand.init)
        case .codex:
            return [
                ("/model", "Choose the model"), ("/approvals", "Approval mode"), ("/status", "Session configuration and usage"),
                ("/compact", "Summarise to free context"), ("/new", "New conversation"), ("/init", "Create AGENTS.md"),
                ("/diff", "Show the git diff"), ("/mention", "Attach a file"), ("/review", "Review the working tree"),
                ("/mcp", "MCP tools"), ("/logout", "Sign out"), ("/quit", "Exit"),
            ].map(AgentSlashCommand.init)
        case .ollama:
            return [
                ("/set", "Set session variables"), ("/show", "Show model information"), ("/load", "Load a model"),
                ("/save", "Save the session as a model"), ("/clear", "Clear session context"), ("/bye", "Exit"), ("/?", "Help"),
            ].map(AgentSlashCommand.init)
        case .antigravity, .grok:
            return [("/help", "List the commands this CLI supports")].map(AgentSlashCommand.init)
        }
    }
}

struct CommandPaletteSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    let tabAgent: AgentType
    let onClose: () -> Void

    @State private var agent: PaletteAgent
    @State private var query = ""
    @State private var custom = ""

    init(viewModel: TerminalSessionViewModel, tabAgent: AgentType, onClose: @escaping () -> Void) {
        self.viewModel = viewModel
        self.tabAgent = tabAgent
        self.onClose = onClose
        _agent = State(initialValue: PaletteAgent(tab: tabAgent))
    }

    private var filtered: [AgentSlashCommand] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return agent.commands.filter { q.isEmpty || $0.name.lowercased().contains(q) || $0.detail.lowercased().contains(q) }
    }

    private func insert(_ text: String) { viewModel.insertAiCommand(text); onClose() }
    private func run(_ text: String) { viewModel.executeAiCommand(text); onClose() }

    var body: some View {
        NavigationStack {
            List {
                if tabAgent == .customShell {
                    Section(footer: Text("This tab runs a plain shell. Slash commands only work inside an agent: start one first (Agents), or pick the one you started by hand.")) {
                        Picker("Agent in this tab", selection: $agent) {
                            ForEach(PaletteAgent.allCases) { Text($0.rawValue).tag($0) }
                        }
                    }
                }
                Section("Any command") {
                    HStack {
                        TextField("/command args", text: $custom)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                        Button("Insert") { insert(custom) }.buttonStyle(.borderless).disabled(custom.isEmpty)
                        Button("Run") { run(custom) }.buttonStyle(.glassProminent).controlSize(.small).disabled(custom.isEmpty)
                    }
                }
                Section(agent.rawValue) {
                    ForEach(filtered) { c in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(c.name).font(.system(size: 15, design: .monospaced))
                                Text(c.detail).font(.caption).foregroundColor(.secondary)
                            }
                            Spacer()
                            Button("Insert") { insert(c.name) }.buttonStyle(.borderless).font(.caption)
                            Button { run(c.name) } label: { Image(systemName: "return") }
                                .buttonStyle(.glassProminent).controlSize(.small)
                                .accessibilityLabel("Run \(c.name)")
                        }
                    }
                }
            }
            .searchable(text: $query, prompt: "Filter commands")
            .navigationTitle("Commands")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
        }
    }
}
