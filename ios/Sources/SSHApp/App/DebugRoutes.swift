import SwiftUI

#if DEBUG
/// Opens one screen directly, so simulator screenshots of any page need no tapping:
/// `xcrun simctl launch <device> com.semenov.sshapp -ui-screen settings-appearance`.
/// Debug builds only; a release build does not contain it.
@MainActor
enum DebugRoutes {
    static var current: String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-ui-screen"), args.indices.contains(i + 1) else { return nil }
        return args[i + 1]
    }

    @ViewBuilder
    static func view(_ route: String) -> some View {
        let servers = ServerProfilesManager()
        switch route {
        case "settings": SettingsView(serversManager: servers)
        case "settings-servers": NavigationStack { ServersSettingsPage(serversManager: servers) }
        case "settings-keys": NavigationStack { KeysSettingsPage(serversManager: servers) }
        case "settings-terminal": NavigationStack { TerminalSettingsPage() }
        case "settings-appearance": NavigationStack { AppearanceSettingsPage() }
        case "settings-modules": NavigationStack { ModuleSettingsView() }
        case "settings-sync": NavigationStack { SyncSettingsView() }
        case "hooks": DebugHooks()
        default: Text("Unknown screen: \(route)")
        }
    }
}

private struct DebugHooks: View {
    @State private var p: SavedAgentProfile = {
        var p = SavedAgentProfile(name: "Claude", agent: .claudeCode, binary: "claude")
        p.hookExtras = [SavedHookExtra(event: "PermissionRequest", command: "logger hook")]
        return p
    }()
    var body: some View { NavigationStack { AgentHooksView(profile: $p, error: nil) } }
}
#endif
