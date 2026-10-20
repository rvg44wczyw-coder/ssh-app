import SwiftUI
import UserNotifications
import Security
import CryptoKit
import SshCoreBridge

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
final class PushAppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    /// A tap on a push: the payload's `sid` says which tab (see `PushManager.sessionId`).
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let sid = response.notification.request.content.userInfo["sid"] as? String
        await MainActor.run { PushManager.shared.pendingSid = sid }
    }

    /// With the app in front the relay's push is not shown (the app makes its own banner for the
    /// same event, see `MainTerminalView.deliverNotices`, and the approval card does the rest);
    /// the app's own local banners are.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions {
        notification.request.trigger is UNPushNotificationTrigger ? [] : [.banner, .list, .sound]
    }

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
    /// The `sid` of a tapped push, until the main screen has opened that tab.
    @Published var pendingSid: String?

    /// Opaque id of a tmux session in the push payload: the first 4 bytes of HMAC-SHA256(relay
    /// secret, "sid:" + session name) in hex. The host computes the same (feature 35), so the
    /// push says nothing about the name to Apple or the relay.
    func sessionId(for sessionName: String) -> String? {
        let secret = relaySecret
        guard !secret.isEmpty else { return nil }
        let mac = HMAC<SHA256>.authenticationCode(for: Data("sid:\(sessionName)".utf8),
                                                  using: SymmetricKey(data: Data(secret.utf8)))
        return mac.prefix(4).map { String(format: "%02x", $0) }.joined()
    }

    /// The owner's relay Worker. The address is not a secret.
    private static let defaultRelayURL = "https://ssh-app-apns-bridge.dgtny5wyr2.workers.dev/push"
    private static let urlKey = "push_relay_url"
    private static let service = "com.semenov.sshapp.push"

    var relayURL: String {
        get { UserDefaults.standard.string(forKey: Self.urlKey) ?? Self.defaultRelayURL }
        set { UserDefaults.standard.set(newValue.trimmingCharacters(in: .whitespacesAndNewlines), forKey: Self.urlKey); objectWillChange.send() }
    }

    var relaySecret: String {
        // Typed value first; else the one `make relay-config` put in this personal build.
        get { Self.keychainRead() ?? LocalRelayConfig.secret }
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

    func didRegister(token: String) {
        self.token = token
        lastError = nil
    }

    /// Hosts the user registered for push once; only these are kept in sync automatically, so the
    /// relay secret never goes to a host the user did not choose.
    private static let hostsKey = "push_registered_hosts"
    private var registeredHosts: Set<String> {
        get { Set(UserDefaults.standard.stringArray(forKey: Self.hostsKey) ?? []) }
        set { UserDefaults.standard.set(Array(newValue).sorted(), forKey: Self.hostsKey) }
    }
    func markRegistered(_ hostKey: String) { registeredHosts.insert(hostKey) }
    func forget(_ hostKey: String) { registeredHosts.remove(hostKey) }

    /// On connect: if this host was registered before and its stored token or environment no
    /// longer match this install, register again.
    func syncIfRegistered(viewModel: TerminalSessionViewModel) async {
        let key = viewModel.hostCacheKey
        guard registeredHosts.contains(key), let token, !relayURL.isEmpty, !relaySecret.isEmpty,
              let handle = viewModel.sessionHandle, let pin = await viewModel.helperPin() else { return }
        let env = Self.apsEnvironment
        if let st = try? await handle.hostPushStatus(expectedSha256: pin), st.registered,
           st.tokenTail == String(token.suffix(6)), st.env == env { return }
        _ = try? await handle.hostPushRegister(expectedSha256: pin, token: token, env: env, relayUrl: relayURL, relaySecret: relaySecret)
    }
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
