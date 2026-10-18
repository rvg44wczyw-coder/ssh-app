import SwiftUI

/// Edit the command buttons of the bar above the keyboard.
struct QuickCommandsEditorView: View {
    /// Long-press menu settings of one command.
    @ViewBuilder
    private func menuEditor(_ cmd: Binding<QuickCommand>) -> some View {
        let source = Binding<String>(
            get: { cmd.wrappedValue.menu?.source.rawValue ?? "none" },
            set: { raw in
                if let s = QuickCommandMenu.Source(rawValue: raw) {
                    let old = cmd.wrappedValue.menu
                    cmd.wrappedValue.menu = QuickCommandMenu(
                        source: s,
                        template: old?.template.isEmpty == false ? old!.template : cmd.wrappedValue.text + " {choice}",
                        items: old?.items ?? "")
                } else {
                    cmd.wrappedValue.menu = nil
                }
            })
        Picker("Long press", selection: source) {
            Text("Nothing").tag("none")
            Text("Models of the tab's agent").tag(QuickCommandMenu.Source.agentModels.rawValue)
            Text("My list").tag(QuickCommandMenu.Source.fixed.rawValue)
        }
        .font(.footnote)
        if let menu = cmd.wrappedValue.menu {
            TextField("Template, e.g. /model {choice}", text: Binding(
                get: { menu.template },
                set: { cmd.wrappedValue.menu?.template = $0 }))
                .font(.system(.footnote, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            if menu.source == .fixed {
                TextEditor(text: Binding(
                    get: { menu.items },
                    set: { cmd.wrappedValue.menu?.items = $0 }))
                    .font(.system(.footnote, design: .monospaced))
                    .frame(minHeight: 60)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Text("One per line: a value, or “label = value”. Letters, digits and . _ : / @ - only.")
                    .font(.caption2).foregroundColor(.secondary)
            }
        }
    }

    @ObservedObject private var store = QuickCommandStore.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach($store.commands) { $cmd in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                TextField("Label", text: $cmd.label)
                                    .font(.system(.body, design: .monospaced).weight(.semibold))
                                Toggle("⏎", isOn: $cmd.run).labelsHidden().fixedSize()
                            }
                            Toggle("Ask before sending", isOn: $cmd.confirm)
                                .font(.footnote)
                            TextField("Command", text: $cmd.text)
                                .font(.system(.footnote, design: .monospaced))
                                .foregroundColor(.secondary)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                            menuEditor($cmd)
                        }
                    }
                    .onDelete { store.commands.remove(atOffsets: $0) }
                    .onMove { store.commands.move(fromOffsets: $0, toOffset: $1) }
                    Button {
                        store.commands.append(QuickCommand(label: "new", text: ""))
                    } label: {
                        Label("Add command", systemImage: "plus")
                    }
                } header: {
                    Text("Buttons above the keyboard")
                } footer: {
                    Text("The ⏎ switch sends Return after the command; “Ask before sending” shows a confirmation on tap. Press and hold a button to open its menu (the /model button lists the models of the tab's agent). Put {{clipboard}} in a command to paste the clipboard text there (the 📋 button is just that). Esc, Ctrl, Tab and arrows are on the system keyboard bar.")
                }
                Section {
                    Button("Reset to defaults", role: .destructive) { store.resetToDefaults() }
                }
            }
            .navigationTitle("Quick commands")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { EditButton() }
                ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } }
            }
        }
        .interactiveDismissDisabled()
    }
}
