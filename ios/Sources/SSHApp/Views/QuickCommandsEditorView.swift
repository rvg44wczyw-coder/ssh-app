import SwiftUI

/// Edit the command buttons of the bar above the keyboard.
struct QuickCommandsEditorView: View {
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
                    Text("The ⏎ switch sends Return after the command; “Ask before sending” shows a confirmation on tap. Put {{clipboard}} in a command to paste the clipboard text there (the 📋 button is just that). Esc, Ctrl, Tab and arrows are on the system keyboard bar.")
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
