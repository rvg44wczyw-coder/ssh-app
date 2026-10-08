import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

public struct KeyboardAccessoryView: View {
    @Binding public var isCtrlLocked: Bool
    public let onSendBytes: ([UInt8]) -> Void
    public let onAiAssistantTap: (() -> Void)?

    public init(
        isCtrlLocked: Binding<Bool>,
        onSendBytes: @escaping ([UInt8]) -> Void,
        onAiAssistantTap: (() -> Void)? = nil
    ) {
        self._isCtrlLocked = isCtrlLocked
        self.onSendBytes = onSendBytes
        self.onAiAssistantTap = onAiAssistantTap
    }

    public var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                // AI Shell Assistant
                if let onAiTap = onAiAssistantTap {
                    AccessoryButton(label: "🪄") {
                        triggerHaptic()
                        onAiTap()
                    }

                    Divider().frame(height: 24)
                }

                // Modifiers
                AccessoryButton(label: "ESC") {
                    triggerHaptic()
                    onSendBytes([0x1B]) // ESC byte
                }

                AccessoryButton(label: "TAB") {
                    triggerHaptic()
                    onSendBytes([0x09]) // TAB byte
                }

                AccessoryToggleButton(label: "CTRL", isActive: isCtrlLocked) {
                    triggerHaptic()
                    isCtrlLocked.toggle()
                }

                AccessoryButton(label: "⏎") {
                    triggerHaptic()
                    onSendBytes([0x0D]) // Enter / Return
                }

                AccessoryButton(label: "⌫") {
                    triggerHaptic()
                    onSendBytes([0x7F]) // Backspace
                }

                AccessoryButton(label: "SPACE") {
                    triggerHaptic()
                    onSendBytes([0x20]) // Space
                }

                Divider().frame(height: 24)

                // Navigation Arrows
                AccessoryButton(label: "▲") {
                    triggerHaptic()
                    onSendBytes([0x1B, 0x5B, 0x41]) // Up arrow
                }
                AccessoryButton(label: "▼") {
                    triggerHaptic()
                    onSendBytes([0x1B, 0x5B, 0x42]) // Down arrow
                }
                AccessoryButton(label: "◄") {
                    triggerHaptic()
                    onSendBytes([0x1B, 0x5B, 0x44]) // Left arrow
                }
                AccessoryButton(label: "►") {
                    triggerHaptic()
                    onSendBytes([0x1B, 0x5B, 0x43]) // Right arrow
                }

                Divider().frame(height: 24)

                // Common control signals
                AccessoryButton(label: "^C") {
                    triggerHaptic()
                    onSendBytes([0x03]) // SIGINT
                }

                AccessoryButton(label: "^D") {
                    triggerHaptic()
                    onSendBytes([0x04]) // EOF
                }

                Divider().frame(height: 24)

                // Agent Quick Action Buttons
                AccessoryButton(label: "✓ y⏎") {
                    triggerHaptic()
                    onSendBytes([0x79, 0x0D]) // y + Enter
                }

                AccessoryButton(label: "✗ n⏎") {
                    triggerHaptic()
                    onSendBytes([0x6E, 0x0D]) // n + Enter
                }

                Divider().frame(height: 24)

                // Quick Symbols for terminal and commands
                Group {
                    symbolButton("/")
                    symbolButton("-")
                    symbolButton("|")
                    symbolButton("~")
                    symbolButton("_")
                    symbolButton("$")
                    symbolButton(":")
                    symbolButton(";")
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
        .background(Color.appSecondaryBackground)
    }

    private func symbolButton(_ char: String) -> some View {
        AccessoryButton(label: char) {
            triggerHaptic()
            if let ascii = char.utf8.first {
                onSendBytes([ascii])
            }
        }
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

struct AccessoryToggleButton: View {
    let label: String
    let isActive: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 13, weight: .bold, design: .monospaced))
                .foregroundColor(isActive ? .white : .primary)
                .frame(minWidth: 42, minHeight: 34)
                .padding(.horizontal, 6)
                .background(isActive ? Color.accentColor : Color.appTertiaryBackground)
                .cornerRadius(6)
                .shadow(color: Color.black.opacity(0.15), radius: 1, y: 1)
        }
    }
}
