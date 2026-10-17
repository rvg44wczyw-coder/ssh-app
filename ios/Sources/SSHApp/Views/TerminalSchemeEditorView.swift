import SwiftUI

/// Create or change a terminal color scheme: name, background, text, cursor and the 16 ANSI
/// colors, with a live preview. "Start from" copies the colors of any existing scheme.
struct TerminalSchemeEditorView: View {
    @State private var scheme: TerminalScheme
    var onSave: (TerminalScheme) -> Void
    var onCancel: () -> Void

    init(scheme: TerminalScheme, onSave: @escaping (TerminalScheme) -> Void, onCancel: @escaping () -> Void) {
        _scheme = State(initialValue: scheme)
        self.onSave = onSave
        self.onCancel = onCancel
    }

    private static let ansiNames = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]

    private func binding(_ keyPath: WritableKeyPath<TerminalScheme, String>) -> Binding<Color> {
        Binding(
            get: { Color(hex: scheme[keyPath: keyPath]) },
            set: { scheme[keyPath: keyPath] = $0.hexString }
        )
    }

    private func ansiBinding(_ i: Int) -> Binding<Color> {
        Binding(
            get: { Color(hex: scheme.ansi[i]) },
            set: { scheme.ansi[i] = $0.hexString }
        )
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Name") {
                    TextField("Scheme name", text: $scheme.name)
                    Menu {
                        ForEach(TerminalScheme.all) { base in
                            Button(base.name) {
                                scheme = TerminalScheme(id: scheme.id, name: scheme.name, background: base.background,
                                                        foreground: base.foreground, cursor: base.cursor, ansi: base.ansi)
                            }
                        }
                    } label: {
                        Label("Start from another scheme", systemImage: "doc.on.doc")
                    }
                }
                Section("Preview") {
                    VStack(alignment: .leading, spacing: 3) {
                        Group {
                            Text("ivs@mac ~/rust/ssh-app % claude").foregroundColor(Color(hex: scheme.foreground))
                            HStack(spacing: 0) {
                                ForEach(0..<8, id: \.self) { Text("██").foregroundColor(Color(hex: scheme.ansi[$0])) }
                            }
                            HStack(spacing: 0) {
                                ForEach(8..<16, id: \.self) { Text("██").foregroundColor(Color(hex: scheme.ansi[$0])) }
                            }
                            HStack(spacing: 0) {
                                Text("error: ").foregroundColor(Color(hex: scheme.ansi[1]))
                                Text("ok ").foregroundColor(Color(hex: scheme.ansi[2]))
                                Text("warn ").foregroundColor(Color(hex: scheme.ansi[3]))
                                Text("info ").foregroundColor(Color(hex: scheme.ansi[4]))
                                Text("▌").foregroundColor(Color(hex: scheme.cursor))
                            }
                        }
                        .font(.system(size: 13, design: .monospaced))
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color(hex: scheme.background))
                    .cornerRadius(8)
                    .listRowInsets(EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12))
                }
                Section("Main colors") {
                    ColorPicker("Background", selection: binding(\.background), supportsOpacity: false)
                    ColorPicker("Text", selection: binding(\.foreground), supportsOpacity: false)
                    ColorPicker("Cursor", selection: binding(\.cursor), supportsOpacity: false)
                }
                Section("Normal colors") {
                    ForEach(0..<8, id: \.self) { i in
                        ColorPicker(Self.ansiNames[i].capitalized, selection: ansiBinding(i), supportsOpacity: false)
                    }
                }
                Section("Bright colors") {
                    ForEach(0..<8, id: \.self) { i in
                        ColorPicker("Bright \(Self.ansiNames[i])", selection: ansiBinding(8 + i), supportsOpacity: false)
                    }
                }
            }
            .navigationTitle("Color scheme")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: onCancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { onSave(scheme) }
                        .disabled(scheme.name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .interactiveDismissDisabled()
    }
}
