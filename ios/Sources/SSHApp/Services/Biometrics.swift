import Foundation
import LocalAuthentication

/// Face ID with the device passcode as the fallback, so a broken Face ID cannot lock the owner
/// out (feature 56 section 6). Used before turning on key sync and, later, for the app lock.
enum Biometrics {
    /// `true` when the owner passed the check. Without any device authentication set up it is
    /// `false`, so keys never leave the device on a phone that has no passcode.
    @MainActor
    static func authenticate(reason: String) async -> Bool {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else { return false }
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) ?? false
    }
}
