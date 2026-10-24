import SwiftUI

/// Settings → Security: the app lock (feature 56 section 6).
struct SecuritySettingsPage: View {
    @AppStorage(AppLock.enabledKey) private var enabled = false
    @AppStorage(AppLock.afterKey) private var after = 0
    @AppStorage(AppLock.hideKey) private var hide = true
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
                Text("Face ID, then the device passcode, so a broken Face ID cannot lock you out. A lock screen covers everything, open sheets included. Sessions reconnect behind it as usual. Exporting keys and changing the vault ask again. Notifications and the Files extension are not covered.")
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
