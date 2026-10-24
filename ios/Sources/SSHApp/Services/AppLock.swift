import SwiftUI
import UIKit
import LocalAuthentication

/// The app lock (feature 56 section 6): Face ID with the device passcode as the fallback. A lock
/// screen in its own window covers everything, open sheets included, and the same window hides the
/// content from the App Switcher picture. The SSH socket lifecycle is not touched: the app
/// reconnects under the lock screen as it always does.
@MainActor
final class AppLock: ObservableObject {
    static let shared = AppLock()

    static let enabledKey = "app_lock_enabled"
    static let afterKey = "app_lock_after_seconds"
    static let hideKey = "app_lock_hide_switcher"
    /// "Lock after" choices in seconds; 0 is immediately.
    static let afterChoices: [Int] = [0, 60, 300, 900]

    @Published private(set) var locked = false
    private var backgroundedAt: Date?
    private var window: UIWindow?
    private var authenticating = false
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var enabled: Bool { defaults.bool(forKey: Self.enabledKey) }
    var lockAfter: TimeInterval { TimeInterval(defaults.object(forKey: Self.afterKey) as? Int ?? 0) }
    /// On unless the owner switched it off.
    var hidesInSwitcher: Bool { defaults.object(forKey: Self.hideKey) as? Bool ?? true }

    /// Pure rule: a relaunch always locks; a return from the background locks once the chosen time
    /// has passed.
    nonisolated static func shouldLock(enabled: Bool, backgroundedAt: Date?, now: Date, after: TimeInterval) -> Bool {
        guard enabled else { return false }
        guard let backgroundedAt else { return true }
        return now.timeIntervalSince(backgroundedAt) >= after
    }

    /// Turning the lock on needs a way to unlock: a phone without a passcode cannot use it.
    static var canBeUsed: Bool {
        var error: NSError?
        return LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
    }

    /// Called once when the app starts: locked at once if the lock is on.
    func launch() {
        guard enabled, !locked, window == nil else { return }
        lock()
    }

    func scenePhaseChanged(_ phase: ScenePhase) {
        guard enabled else {
            if window != nil, !locked { hideWindow() }
            return
        }
        switch phase {
        case .inactive:
            // The App Switcher picture is taken now: cover the content before it is.
            if !locked, hidesInSwitcher { showWindow(cover: true) }
        case .background:
            if backgroundedAt == nil { backgroundedAt = Date() }
            if !locked, hidesInSwitcher { showWindow(cover: true) }
        case .active:
            let was = backgroundedAt
            backgroundedAt = nil
            if locked { return }
            if Self.shouldLock(enabled: true, backgroundedAt: was, now: Date(), after: lockAfter), was != nil {
                lock()
            } else {
                hideWindow()
            }
        @unknown default:
            break
        }
    }

    private func lock() {
        locked = true
        showWindow(cover: false)
    }

    /// Asks Face ID (or the passcode). Without any device authentication the lock cannot protect
    /// anything, so the app opens rather than locking the owner out for good.
    func unlock() async {
        guard locked, !authenticating else { return }
        authenticating = true
        defer { authenticating = false }
        var ok = !Self.canBeUsed
        if !ok { ok = await Biometrics.authenticate(reason: "Unlock SSH-App") }
        if ok {
            locked = false
            hideWindow()
        }
    }

    /// An extra check before something sensitive (exporting keys, showing a key): the owner is asked
    /// again whatever the lock timer says.
    func confirmOwner(reason: String) async -> Bool {
        await Biometrics.authenticate(reason: reason)
    }

    // MARK: Window

    private func showWindow(cover: Bool) {
        if let window {
            window.isHidden = false
            return
        }
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first else { return }
        let w = UIWindow(windowScene: scene)
        w.windowLevel = .alert + 10
        let host = UIHostingController(rootView: LockScreenView(lock: self))
        host.view.backgroundColor = .clear
        w.rootViewController = host
        w.isHidden = false
        window = w
    }

    private func hideWindow() {
        window?.isHidden = true
        window = nil
    }
}

struct LockScreenView: View {
    @ObservedObject var lock: AppLock

    var body: some View {
        ZStack {
            Rectangle().fill(.background).ignoresSafeArea()
            VStack(spacing: 18) {
                Image(systemName: "lock.fill").font(.system(size: 44)).foregroundStyle(.secondary)
                Text("SSH-App is locked").font(.headline)
                if lock.locked {
                    Button { Task { await lock.unlock() } } label: {
                        Label("Unlock", systemImage: "faceid").padding(.horizontal, 12)
                    }
                    .buttonStyle(.glassProminent)
                    .controlSize(.large)
                }
            }
        }
        .task(id: lock.locked) {
            if lock.locked { await lock.unlock() }
        }
    }
}
