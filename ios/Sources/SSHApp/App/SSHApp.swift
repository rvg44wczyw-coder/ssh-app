import SwiftUI

@main
public struct SSHMobileApp: App {
    public init() {}

    public var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}

/// Applies the chosen accent colour to the whole app (controls, links, selection).
private struct RootView: View {
    @AppStorage(AppAccent.storageKey) private var accentRaw: String = AppAccent.blue.rawValue

    var body: some View {
        MainTerminalView()
            .tint((AppAccent(rawValue: accentRaw) ?? .blue).color)
    }
}
