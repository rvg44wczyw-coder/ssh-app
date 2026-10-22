import XCTest
import SshCoreBridge
@testable import SSHApp

final class VaultSyncTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_000)

    private func profile(_ name: String, host: String = "h.example", at seconds: Double, fp: String? = nil, key: String? = nil) -> ServerProfile {
        ServerProfile(name: name, host: host, port: 22, username: "me", hostKeyFingerprint: fp, keyId: key,
                      updatedAt: Date(timeIntervalSince1970: seconds))
    }

    private func remote(of profiles: [ServerProfile], fingerprints: Bool = false) -> [VaultEntry] {
        VaultSyncEngine.entries(for: profiles, tombstones: [:], syncFingerprints: fingerprints)
    }

    func testANewServerFromTheOtherDeviceArrivesWithoutKeyOrDefault() {
        let theirs = profile("Mac", at: 100, key: "their-key")
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(out.profiles.count, 1)
        XCTAssertEqual(out.profiles[0].id, theirs.id)
        XCTAssertNil(out.profiles[0].keyId, "each device signs with its own key")
        XCTAssertFalse(out.profiles[0].isDefault)
        XCTAssertTrue(out.profilesChanged)
    }

    func testTheLaterEditWinsAndKeepsLocalKeyChoice() {
        var mine = profile("Old name", at: 100, key: "my-key")
        var theirs = mine
        theirs.name = "New name"; theirs.updatedAt = Date(timeIntervalSince1970: 200)
        let out = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(out.profiles[0].name, "New name")
        XCTAssertEqual(out.profiles[0].keyId, "my-key")
        mine.updatedAt = Date(timeIntervalSince1970: 300)
        let kept = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(kept.profiles[0].name, "Old name")
        XCTAssertFalse(kept.profilesChanged)
    }

    func testADeletionOnTheOtherDeviceRemovesTheServerHere() {
        let p = profile("Gone", at: 100)
        let stone = VaultEntry(id: p.id.uuidString, updatedAtMs: 200_000, deleted: true, payload: "")
        let out = VaultSyncEngine.reconcile(local: [p], tombstones: [:], remote: [stone], syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
        XCTAssertEqual(out.tombstones[p.id.uuidString], 200_000)
    }

    func testALocalDeletionIsNotUndoneByAnOlderRemoteCopy() {
        let p = profile("Gone", at: 100)
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [p.id.uuidString: 300_000], remote: remote(of: [p]), syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
    }

    func testFingerprintsTravelOnlyWithTheirToggleAndADifferentOneIsAConflict() {
        let theirs = profile("Mac", at: 200, fp: "SHA256:theirs")
        XCTAssertNil(VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now).profiles.first?.hostKeyFingerprint)
        XCTAssertFalse(remote(of: [theirs]).first!.payload.contains("SHA256"))

        let onWire = remote(of: [theirs], fingerprints: true)
        let fresh = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: onWire, syncFingerprints: true, now: now)
        XCTAssertEqual(fresh.profiles[0].hostKeyFingerprint, "SHA256:theirs")

        var mine = theirs; mine.hostKeyFingerprint = "SHA256:mine"; mine.updatedAt = Date(timeIntervalSince1970: 100)
        let clash = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: onWire, syncFingerprints: true, now: now)
        XCTAssertEqual(clash.profiles[0].hostKeyFingerprint, "SHA256:mine", "a pin is never replaced silently")
        XCTAssertEqual(clash.fingerprintConflicts, ["Mac"])
    }

    func testHostileRemotePayloadsNeverBecomeProfiles() {
        let bad = [
            #"{"name":"x","host":"-oProxyCommand=evil","port":22,"username":"me"}"#,
            #"{"name":"x","host":"a b","port":22,"username":"me"}"#,
            #"{"name":"x","host":"h","port":0,"username":"me"}"#,
            #"{"name":"x","host":"h","port":22,"username":"-l root"}"#,
            "not json",
        ]
        let entries = bad.map { VaultEntry(id: UUID().uuidString, updatedAtMs: 5, deleted: false, payload: $0) }
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: entries, syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
    }

    func testThePayloadCarriesNoKeyChoiceOrDefaultFlag() {
        let p = profile("Mac", at: 100, key: "secret-key-id")
        let wire = remote(of: [p])
        XCTAssertEqual(wire.count, 1)
        XCTAssertFalse(wire[0].payload.contains("secret-key-id"))
        XCTAssertFalse(wire[0].payload.contains("isDefault"))
    }
}

final class KeyImportTests: XCTestCase {
    func testAnEncryptedKeyNeedsItsPassphraseAndTheWrongOneIsTyped() throws {
        let pair = try generateSshKeypair()
        XCTAssertFalse(try privateKeyNeedsPassphrase(armoredKey: pair.privateKeyOpenssh))
        let imported = try importPrivateKey(armoredKey: pair.privateKeyOpenssh, passphrase: nil)
        XCTAssertEqual(imported.publicKeyOpenssh.trimmingCharacters(in: .whitespacesAndNewlines),
                       pair.publicKeyOpenssh.trimmingCharacters(in: .whitespacesAndNewlines))
        XCTAssertThrowsError(try importPrivateKey(armoredKey: "garbage", passphrase: nil))
    }
}

final class HostScreensTests: XCTestCase {
    func testHostScreensAreOwnQuickMenuTilesThatNeedAConnection() {
        for a in [AppAction.telemetry, .processes, .docker, .hostHelper] {
            XCTAssertTrue(a.requiresConnection)
            XCTAssertTrue(a.canHide)
            XCTAssertFalse(a.isSessionAction)
        }
        // An order saved before these tiles existed still lists them, at the end.
        let old = QuickMenuPrefs.raw(from: [.settings, .telemetry])
        let order = QuickMenuPrefs.ordered(from: old)
        XCTAssertEqual(Array(order.prefix(2)), [.settings, .telemetry])
        XCTAssertTrue(order.contains(.docker) && order.contains(.processes) && order.contains(.hostHelper))
    }

    func testEachUnavailableReasonHasItsOwnWording() {
        let snap = parseDockerList(output: "DOCKER_NOT_INSTALLED\n")
        XCTAssertEqual(snap.unavailable, .notInstalled)
        XCTAssertEqual(parseDockerList(output: "permission denied while trying to connect to the Docker daemon socket").unavailable, .noPermission)
        XCTAssertEqual(parseDockerList(output: "Cannot connect to the Docker daemon. Is the docker daemon running?").unavailable, .daemonDown)
        XCTAssertEqual(parseDockerList(output: "").unavailable, nil)
    }
}
