import SwiftUI

/// Settings → Security: the app lock (feature 56 section 6).
struct SecuritySettingsPage: View {
    @AppStorage(AppLock.enabledKey) private var enabled = false
    @AppStorage(AppLock.afterKey) private var after = 0
    @AppStorage(AppLock.hideKey) private var hide = true
    @AppStorage(FilesIntegration.requireAuthKey) private var filesAuth = false
    @State private var unavailable = false

    var body: some View {
        Form {
            Section {
                Toggle("Require Face ID", isOn: Binding(
                    get: { enabled },
                    set: { on in
                        guard on else { enabled = false; return }
                        guard AppLock.canBeUsed else { unavailable = true; return }
                        Task { if await AppLock.shared.confirmOwner(reason: "Confirm to turn on the app lock") { enabled = true } }
                    }))
                if enabled {
                    Picker("Lock after", selection: $after) {
                        ForEach(AppLock.afterChoices, id: \.self) { secs in
                            Text(secs == 0 ? "Immediately" : "\(secs / 60) min").tag(secs)
                        }
                    }
                    Toggle("Hide in the app switcher", isOn: $hide)
                }
            } footer: {
                Text("Face ID, then the device passcode, so a broken Face ID cannot lock you out. A lock screen covers everything, open sheets included. Sessions reconnect behind it as usual. Exporting keys and changing the vault ask again. Notifications are not covered.")
            }
            Section {
                Toggle("Require Face ID for Files", isOn: Binding(
                    get: { filesAuth },
                    set: { on in
                        guard on else { filesAuth = false; FilesIntegration.reapply(); return }
                        guard AppLock.canBeUsed else { unavailable = true; return }
                        Task {
                            if await AppLock.shared.confirmOwner(reason: "Confirm to protect the servers in Files") {
                                filesAuth = true
                                FilesIntegration.reapply()
                            }
                        }
                    }))
            } footer: {
                Text("The Files app then asks for Face ID (or the passcode) when it opens a server. The key it uses is a separate copy that only this check unlocks. If the check fails, the server does not open.")
            }
        }
        .navigationTitle("Security")
        .alert("Set a device passcode first", isPresented: $unavailable) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("The lock uses the device passcode as its fallback.")
        }
    }
}
