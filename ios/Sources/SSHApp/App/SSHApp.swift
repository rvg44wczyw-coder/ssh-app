import SwiftUI
import UserNotifications
import Security

@main
public struct SSHMobileApp: App {
    @UIApplicationDelegateAdaptor(PushAppDelegate.self) private var appDelegate

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
            .tint(AppAccent.color(for: accentRaw))
    }
}

/// Hands the APNs device token to `PushManager`.
final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { @MainActor in PushManager.shared.didRegister(token: hex) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in PushManager.shared.didFail(error.localizedDescription) }
    }
}

/// Push for "an agent needs you" with the app closed (feature 35). The phone asks for permission,
/// gets a device token and registers it on hosts together with the relay's address and secret.
/// The notifications carry no content; the relay and the host only know four fixed categories.
@MainActor
final class PushManager: ObservableObject {
    static let shared = PushManager()

    @Published private(set) var token: String?
    @Published private(set) var authorization: UNAuthorizationStatus = .notDetermined
    @Published private(set) var lastError: String?

    private static let urlKey = "push_relay_url"
    private static let service = "com.semenov.sshapp.push"

    var relayURL: String {
        get { UserDefaults.standard.string(forKey: Self.urlKey) ?? "" }
        set { UserDefaults.standard.set(newValue.trimmingCharacters(in: .whitespacesAndNewlines), forKey: Self.urlKey); objectWillChange.send() }
    }

    var relaySecret: String {
        get { Self.keychainRead() ?? "" }
        set { Self.keychainWrite(newValue.trimmingCharacters(in: .whitespacesAndNewlines)); objectWillChange.send() }
    }

    /// `sandbox` for development-signed installs, `production` for TestFlight and the App Store.
    /// Read from the embedded provisioning profile; App Store builds have none.
    nonisolated static var apsEnvironment: String {
        guard let path = Bundle.main.path(forResource: "embedded", ofType: "mobileprovision"),
              let data = FileManager.default.contents(atPath: path),
              let text = String(data: data, encoding: .isoLatin1),
              let r = text.range(of: "<key>aps-environment</key>") else { return "production" }
        let tail = text[r.upperBound...].prefix(80)
        return tail.contains("<string>development</string>") ? "sandbox" : "production"
    }

    init() {
        UserDefaults.standard.register(defaults: [:])
        Task { await refreshAuthorization() }
    }

    func refreshAuthorization() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        authorization = settings.authorizationStatus
        if authorization == .authorized { UIApplication.shared.registerForRemoteNotifications() }
    }

    /// Asks for permission (once) and starts the registration with APNs.
    func enable() async {
        lastError = nil
        do {
            let granted = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
            await refreshAuthorization()
            if granted { UIApplication.shared.registerForRemoteNotifications() }
            else { lastError = "Notifications are off for this app in iOS Settings." }
        } catch {
            lastError = error.localizedDescription
        }
    }

    func didRegister(token: String) { self.token = token; lastError = nil }
    func didFail(_ message: String) { lastError = message }

    private static func query() -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "relay-secret"]
    }

    private static func keychainRead() -> String? {
        var q = query()
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data else { return nil }
        return String(data: d, encoding: .utf8)
    }

    private static func keychainWrite(_ value: String) {
        SecItemDelete(query() as CFDictionary)
        guard !value.isEmpty else { return }
        var q = query()
        q[kSecValueData as String] = Data(value.utf8)
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(q as CFDictionary, nil)
    }
}
