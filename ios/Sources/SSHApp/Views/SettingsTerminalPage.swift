import SwiftUI
import SshCoreBridge

/// Settings -> Terminal: assistant tile, start folder, usage line, font size.
struct TerminalSettingsPage: View {
    @AppStorage("terminal_font_size") private var terminalFontSize: Double = 13.0
    @AppStorage(AiAssistantVisibility.storageKey) private var aiVisibilityRaw: String = ""
    @AppStorage("always_show_ai_assistant") private var legacyAlwaysShowAi: Bool = false
    @AppStorage(TerminalFolder.storageKey) private var terminalFolder: String = TerminalFolder.fallback
    @AppStorage("usage_strip_enabled") private var usageStripEnabled: Bool = true

    var body: some View {
        Form {
            Section(
                header: Text("Quick menu"),
                footer: Text("Auto shows it only while connected.")
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
                footer: Text(TerminalFolder.problem(terminalFolder) ?? "Plain terminals start here on the host. Empty = home. Agent tabs use their profile's folder.")
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
                footer: Text("Model, context fill and Claude tokens of the last 5 hours, refreshed every 90 s.")
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
}
