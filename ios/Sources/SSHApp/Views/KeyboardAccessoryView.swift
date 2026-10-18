import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

public struct KeyboardAccessoryView: View {
    public let onSendBytes: ([UInt8]) -> Void
    public let onAiAssistantTap: (() -> Void)?
    public let onSnippetsTap: (() -> Void)?
    @ObservedObject private var store = QuickCommandStore.shared
    @State private var showEditor = false
    @State private var pendingClear: QuickCommand?

    public init(
        onSendBytes: @escaping ([UInt8]) -> Void,
        onAiAssistantTap: (() -> Void)? = nil,
        onSnippetsTap: (() -> Void)? = nil
    ) {
        self.onSendBytes = onSendBytes
        self.onAiAssistantTap = onAiAssistantTap
        self.onSnippetsTap = onSnippetsTap
    }

    /// Esc, Ctrl, Tab, arrows and symbols are already on the system keyboard bar; this bar only
    /// carries what that one lacks: the assistant, snippets, Ctrl-C, paste, y/n and the user's own
    /// command buttons.
    public var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                if let onAiTap = onAiAssistantTap {
                    AccessoryButton(label: "🪄") {
                        triggerHaptic()
                        onAiTap()
                    }
                }
                if let onSnippets = onSnippetsTap {
                    AccessoryButton(label: "⚡") {
                        triggerHaptic()
                        onSnippets()
                    }
                }
                AccessoryButton(label: "^C") {
                    triggerHaptic()
                    onSendBytes([0x03])
                }
                AccessoryButton(label: "📋") {
                    triggerHaptic()
                    pasteFromClipboard()
                }
                AccessoryButton(label: "✓ y⏎") {
                    triggerHaptic()
                    onSendBytes([0x79, 0x0D])
                }
                AccessoryButton(label: "✗ n⏎") {
                    triggerHaptic()
                    onSendBytes([0x6E, 0x0D])
                }

                Divider().frame(height: 24)

                ForEach(store.commands) { cmd in
                    AccessoryButton(label: cmd.label) {
                        triggerHaptic()
                        if cmd.confirm {
                            pendingClear = cmd
                        } else {
                            onSendBytes(cmd.bytes)
                        }
                    }
                }
                AccessoryButton(label: "✎") {
                    triggerHaptic()
                    showEditor = true
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
        .background(Color.appSecondaryBackground)
        .sheet(isPresented: $showEditor) { QuickCommandsEditorView() }
        .confirmationDialog(
            "Run \(pendingClear?.label ?? "")?",
            isPresented: Binding(get: { pendingClear != nil }, set: { if !$0 { pendingClear = nil } }),
            titleVisibility: .visible,
            presenting: pendingClear
        ) { cmd in
            Button("Run \(cmd.label)", role: .destructive) { onSendBytes(cmd.bytes) }
            Button("Cancel", role: .cancel) {}
        } message: { cmd in
            Text("Send \"\(cmd.text)\" to the session?")
        }
    }

    /// Sends the clipboard text to the session; line breaks become Enter, as a terminal paste does.
    private func pasteFromClipboard() {
        #if canImport(UIKit)
        guard UIPasteboard.general.hasStrings, let text = UIPasteboard.general.string, !text.isEmpty
        else { return }
        let normalized = text.replacingOccurrences(of: "\r\n", with: "\r").replacingOccurrences(of: "\n", with: "\r")
        onSendBytes(Array(normalized.utf8))
        #endif
    }

    private func triggerHaptic() {
        #if canImport(UIKit)
        let generator = UIImpactFeedbackGenerator(style: .light)
        generator.prepare()
        generator.impactOccurred()
        #endif
    }
}

struct AccessoryButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 13, weight: .semibold, design: .monospaced))
                .foregroundColor(.primary)
                .frame(minWidth: 36, minHeight: 34)
                .padding(.horizontal, 6)
                .background(Color.appTertiaryBackground)
                .cornerRadius(6)
                .shadow(color: Color.black.opacity(0.1), radius: 1, y: 1)
        }
    }
}
