import XCTest
import SshCoreBridge
@testable import SSHApp

private final class MemoryStore: VaultItemStore {
    var items: [String: Data] = [:]
    func load(_ account: String) -> Data? { items[account] }
    func save(_ data: Data, _ account: String) -> Bool { items[account] = data; return true }
    func delete(_ account: String) { items[account] = nil }
}

private final class MemoryKey: VaultKeyStorage {
    var key: Data?
    func load() -> Data? { key }
    func save(_ key: Data) -> Bool { self.key = key; return true }
    func delete() { key = nil }
}

@MainActor
final class EncryptedVaultTests: XCTestCase {
    private let phrase = "correct horse battery staple"

    private func defaults() -> UserDefaults {
        let d = UserDefaults(suiteName: "vault-test-\(UUID().uuidString)")!
        return d
    }

    private func device(_ store: MemoryStore, now: @escaping () -> Date = Date.init) -> (EncryptedVault, MemoryKey) {
        let key = MemoryKey()
        return (EncryptedVault(store: store, keys: key, defaults: defaults(), now: now), key)
    }

    func testCreateNeedsTwoMatchingStrongPassphrases() {
        let (vault, _) = device(MemoryStore())
        XCTAssertThrowsError(try vault.create(passphrase: phrase, confirm: phrase + "x")) { XCTAssertEqual($0 as? VaultError, .mismatch) }
        XCTAssertThrowsError(try vault.create(passphrase: "short", confirm: "short")) { XCTAssertEqual($0 as? VaultError, .weak) }
        XCTAssertEqual(vault.state, .off)
        XCTAssertNoThrow(try vault.create(passphrase: phrase, confirm: phrase))
        XCTAssertEqual(vault.state, .ready)
        XCTAssertEqual(vault.key?.count, 32)
    }

    func testASecondDeviceMustJoinWithThePassphrase() throws {
        let cloud = MemoryStore()
        let (first, firstKey) = device(cloud)
        try first.create(passphrase: phrase, confirm: phrase)
        let (second, secondKey) = device(cloud)
        XCTAssertEqual(second.state, .needsJoin)
        XCTAssertNil(second.key)
        XCTAssertThrowsError(try second.join(passphrase: "a different long passphrase")) { XCTAssertEqual($0 as? VaultError, .wrongPassphrase) }
        XCTAssertNil(secondKey.key)
        // The delay after a miss is enforced (the clock does not move).
        XCTAssertThrowsError(try second.join(passphrase: phrase)) {
            guard case .tooSoon = $0 as? VaultError else { return XCTFail("expected a delay, got \($0)") }
        }
        XCTAssertEqual(first.key, firstKey.key)
    }

    func testTheDelayEndsAndTheRightPassphraseUnlocksTheSameKey() throws {
        let cloud = MemoryStore()
        let (first, firstKey) = device(cloud)
        try first.create(passphrase: phrase, confirm: phrase)
        var clock = Date()
        let (second, secondKey) = device(cloud, now: { clock })
        XCTAssertThrowsError(try second.join(passphrase: "nope nope nope nope"))
        clock = clock.addingTimeInterval(5)
        try second.join(passphrase: phrase)
        XCTAssertEqual(second.state, .ready)
        XCTAssertEqual(secondKey.key, firstKey.key)
    }

    func testChangingThePassphraseKeepsTheKey() throws {
        let cloud = MemoryStore()
        var clock = Date()
        let (vault, key) = device(cloud, now: { clock })
        try vault.create(passphrase: phrase, confirm: phrase)
        let before = key.key
        XCTAssertThrowsError(try vault.changePassphrase(old: "wrong wrong wrong wrong", new: "another long passphrase", confirm: "another long passphrase"))
        clock = clock.addingTimeInterval(5)
        try vault.changePassphrase(old: phrase, new: "another long passphrase", confirm: "another long passphrase")
        XCTAssertEqual(key.key, before)
        let (other, _) = device(cloud, now: { clock })
        clock = clock.addingTimeInterval(1)
        XCTAssertThrowsError(try other.join(passphrase: phrase), "the old passphrase no longer opens it")
        clock = clock.addingTimeInterval(120)
        XCTAssertNoThrow(try other.join(passphrase: "another long passphrase"))
    }

    func testResetCutsOffDevicesThatHeldTheOldKey() throws {
        let cloud = MemoryStore()
        var clock = Date()
        let (first, _) = device(cloud, now: { clock })
        try first.create(passphrase: phrase, confirm: phrase)
        let (lost, lostKey) = device(cloud, now: { clock })
        try lost.join(passphrase: phrase)
        cloud.items[VaultAccount.secrets] = Data("sealed".utf8)
        clock = clock.addingTimeInterval(10)
        try first.reset(passphrase: "a brand new passphrase", confirm: "a brand new passphrase")
        XCTAssertNil(cloud.items[VaultAccount.secrets], "sealed entries of the old key are deleted")
        lost.refresh()
        XCTAssertEqual(lost.state, .needsJoin)
        XCTAssertNil(lostKey.key, "the old key is dropped")
        XCTAssertNil(lost.key)
    }

    func testADeletedContainerPausesSyncInsteadOfDroppingTheChecks() throws {
        let cloud = MemoryStore()
        let (vault, key) = device(cloud)
        try vault.create(passphrase: phrase, confirm: phrase)
        cloud.items[VaultAccount.container] = nil   // an attacker (or another device) deletes it
        vault.refresh()
        XCTAssertEqual(vault.state, .removed)
        XCTAssertNil(vault.key, "no key is used while the vault is gone")
        XCTAssertNotNil(key.key)
        vault.forget()
        XCTAssertEqual(vault.state, .off)
        XCTAssertNil(key.key)
    }

    func testAReplacedContainerIsReportedAndTheOldKeyDropped() throws {
        let cloud = MemoryStore()
        var clock = Date()
        let (mine, myKey) = device(cloud, now: { clock })
        try mine.create(passphrase: phrase, confirm: phrase)
        clock = clock.addingTimeInterval(100)
        let attacker = MemoryStore()
        let (theirs, _) = device(attacker, now: { clock })
        try theirs.create(passphrase: "attacker chosen passphrase", confirm: "attacker chosen passphrase")
        cloud.items[VaultAccount.container] = attacker.items[VaultAccount.container]
        mine.refresh()
        XCTAssertEqual(mine.state, .needsJoin)
        XCTAssertTrue(mine.replaced)
        XCTAssertNil(myKey.key)
        XCTAssertThrowsError(try mine.join(passphrase: phrase), "the real passphrase does not open it")
    }

    func testTheUnlockDelaySurvivesARelaunch() throws {
        let cloud = MemoryStore()
        let (first, _) = device(cloud)
        try first.create(passphrase: phrase, confirm: phrase)
        let shared = defaults()
        let clock = Date()
        let one = EncryptedVault(store: cloud, keys: MemoryKey(), defaults: shared, now: { clock })
        XCTAssertThrowsError(try one.join(passphrase: "wrong wrong wrong wrong"))
        let relaunched = EncryptedVault(store: cloud, keys: MemoryKey(), defaults: shared, now: { clock })
        XCTAssertThrowsError(try relaunched.join(passphrase: phrase)) {
            guard case .tooSoon = $0 as? VaultError else { return XCTFail("the delay must persist, got \($0)") }
        }
    }

    func testTurningTheVaultOffRemovesItFromICloud() throws {
        let cloud = MemoryStore()
        let (vault, key) = device(cloud)
        try vault.create(passphrase: phrase, confirm: phrase)
        vault.turnOff()
        XCTAssertEqual(vault.state, .off)
        XCTAssertNil(cloud.items[VaultAccount.container])
        XCTAssertNil(key.key)
    }
}

final class VaultWireTests: XCTestCase {
    private let key = Data(repeating: 7, count: 32)
    private func entry(_ id: String, _ payload: String = "{}") -> VaultEntry {
        VaultEntry(id: id, updatedAtMs: 100, deleted: false, payload: payload)
    }

    func testSignedEntriesPassAndForgedOnesAreCounted() throws {
        let data = try XCTUnwrap(VaultWire.encodeSigned([entry("a"), entry("b")], key: key))
        XCTAssertEqual(VaultWire.decodeSigned(data, key: key).entries.count, 2)
        XCTAssertEqual(VaultWire.decodeSigned(data, key: key).rejected, 0)

        let other = Data(repeating: 8, count: 32)
        let wrongKey = VaultWire.decodeSigned(data, key: other)
        XCTAssertEqual(wrongKey.entries.count, 0)
        XCTAssertEqual(wrongKey.rejected, 2)

        // Someone without the key writes an unsigned entry, or edits a signed one.
        var wire = try JSONDecoder().decode([VaultWire.SignedEntry].self, from: data)
        wire[0].payload = "{\"host\":\"evil\"}"
        wire.append(VaultWire.SignedEntry(id: "c", at: 100, deleted: false, payload: "{}", mac: nil))
        let tampered = try JSONEncoder().encode(wire)
        let result = VaultWire.decodeSigned(tampered, key: key)
        XCTAssertEqual(result.entries.map(\.id), ["b"])
        XCTAssertEqual(result.rejected, 2)
    }

    func testWithoutAVaultNothingIsChecked() throws {
        let data = try XCTUnwrap(VaultWire.encodeSigned([entry("a")], key: nil))
        XCTAssertEqual(VaultWire.decodeSigned(data, key: nil).entries.count, 1)
        XCTAssertEqual(VaultWire.decodeSigned(Data("not json".utf8), key: key).entries.count, 0)
    }

    func testSealedEntriesOpenOnlyWithTheirKeyAndId() throws {
        let data = try XCTUnwrap(VaultWire.encodeSealed([entry("k_one", "{\"name\":\"x\"}")], key: key))
        let opened = VaultWire.decodeSealed(data, key: key)
        XCTAssertEqual(opened.entries, [entry("k_one", "{\"name\":\"x\"}")])
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("name"), "the payload is not readable in iCloud")
        XCTAssertEqual(VaultWire.decodeSealed(data, key: Data(repeating: 9, count: 32)).rejected, 1)
        var wire = try JSONDecoder().decode([VaultWire.SealedEntry].self, from: data)
        wire[0].id = "k_two"
        let moved = try JSONEncoder().encode(wire)
        XCTAssertEqual(VaultWire.decodeSealed(moved, key: key).rejected, 1, "an entry cannot be moved under another id")
    }
}

final class SecretSyncEngineTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_000)
    private func item(_ id: String, _ payload: String) -> SecretItem { SecretItem(id: id, label: id, payload: payload) }
    private func entry(_ id: String, _ payload: String, at: UInt64) -> VaultEntry {
        VaultEntry(id: id, updatedAtMs: at, deleted: false, payload: payload)
    }
    private func run(local: [SecretItem], remote: [VaultEntry], state: SecretState,
                     scope: @escaping (String) -> Bool = { _ in true }) -> SecretSyncEngine.Outcome {
        SecretSyncEngine.reconcile(local: local, remote: remote, state: state, now: now, inScope: scope, accept: { _ in true })
    }

    func testOnlyItemsTheUserTurnedOnAreUploaded() {
        var st = SecretState()
        st.enabled = ["k_a"]
        let out = run(local: [item("k_a", "A"), item("k_b", "B")], remote: [], state: st)
        XCTAssertEqual(out.remote.map(\.id), ["k_a"])
    }

    func testAnItemFromAnotherDeviceIsImportedAndThenStaysSynced() {
        let out = run(local: [], remote: [entry("k_x", "X", at: 5)], state: SecretState())
        XCTAssertEqual(out.upserts.map(\.id), ["k_x"])
        XCTAssertTrue(out.state.enabled.contains("k_x"))
        XCTAssertEqual(out.remote.map(\.id), ["k_x"])
    }

    func testADismissedItemIsNotImportedAgain() {
        var st = SecretState()
        st.dismissed = ["k_x"]
        let out = run(local: [], remote: [entry("k_x", "X", at: 5)], state: st)
        XCTAssertTrue(out.upserts.isEmpty)
        XCTAssertEqual(out.remote.map(\.id), ["k_x"], "other devices' copy is left alone")
    }

    func testTurningAnItemOffRemovesItsCopyOnlyOnce() {
        var st = SecretState()
        st.drop = ["k_a"]
        let out = run(local: [item("k_a", "A")], remote: [entry("k_a", "A", at: 5)], state: st)
        XCTAssertTrue(out.remote.isEmpty)
        XCTAssertTrue(out.state.drop.isEmpty)
        XCTAssertTrue(out.upserts.isEmpty)
    }

    func testDeletingTheItemLocallyRemovesTheSharedCopy() {
        var st = SecretState()
        st.enabled = ["k_a"]
        st.payloads["k_a"] = "A"
        let out = run(local: [], remote: [entry("k_a", "A", at: 5)], state: st)
        XCTAssertTrue(out.remote.isEmpty)
        XCTAssertTrue(out.state.dismissed.contains("k_a"))
        XCTAssertTrue(out.upserts.isEmpty, "it is not brought back")
    }

    func testTheLaterEditWins() {
        var st = SecretState()
        st.enabled = ["k_a"]
        st.payloads["k_a"] = "old"
        st.stamps["k_a"] = 10
        let newer = run(local: [item("k_a", "old")], remote: [entry("k_a", "theirs", at: 99)], state: st)
        XCTAssertEqual(newer.upserts.map(\.payload), ["theirs"])
        let mine = run(local: [item("k_a", "edited")], remote: [entry("k_a", "theirs", at: 5)], state: st)
        XCTAssertTrue(mine.upserts.isEmpty)
        XCTAssertEqual(mine.remote.first?.payload, "edited")
    }

    func testACategoryThatIsSwitchedOffIsLeftExactlyAsItIs() {
        var st = SecretState()
        st.enabled = ["k_a", "e_b"]
        let remote = [entry("k_a", "A", at: 5), entry("e_b", "B", at: 5)]
        let out = run(local: [], remote: remote, state: st, scope: { $0.hasPrefix("e_") })
        XCTAssertTrue(out.remote.contains { $0.id == "k_a" })
        XCTAssertTrue(out.state.enabled.contains("k_a"), "an out-of-scope id is not treated as deleted")
        XCTAssertFalse(out.remote.contains { $0.id == "e_b" }, "the in-scope one vanished locally")
    }

    func testIncomingPayloadsAreValidated() {
        let known: Set<String> = ["p1"]
        func env(_ name: String, _ value: String, profile: String = "p1") -> SecretItem {
            let json = String(data: try! JSONEncoder().encode(SyncedEnv(profile: profile, name: name, value: value)), encoding: .utf8)!
            return SecretItem(id: SecretIDs.env(profile: profile, name: name), label: "", payload: json)
        }
        func ok(_ i: SecretItem) -> Bool { VaultSecretItems.accepts(i, knownProfiles: known, hasDefaultKey: false) }
        XCTAssertTrue(ok(env("API_TOKEN", "abc")))
        XCTAssertFalse(ok(env("PATH", "/bin")), "not a secret name")
        XCTAssertFalse(ok(env("API_TOKEN", "line\nbreak")))
        XCTAssertFalse(ok(env("API TOKEN", "x")))
        XCTAssertFalse(ok(env("API_TOKEN", "x", profile: "unknown")), "waits until the profile exists")
        var wrongId = env("API_TOKEN", "abc")
        wrongId.id = SecretIDs.env(profile: "p1", name: "OTHER_TOKEN")
        XCTAssertFalse(ok(wrongId), "the id must match the content")
        XCTAssertFalse(ok(SecretItem(id: "k_default", label: "", payload: "{\"name\":\"x\",\"privateKey\":\"nope\",\"publicKey\":\"nope\"}")))
        XCTAssertFalse(ok(SecretItem(id: "z_other", label: "", payload: "{}")))
    }

    func testAPrivateKeyIsAcceptedOnlyWhenItMatchesItsPublicHalf() throws {
        let pair = try generateSshKeypair()
        func key(_ pub: String, id: String = "k_default") -> SecretItem {
            let json = String(data: try! JSONEncoder().encode(SyncedKey(name: "Work", privateKey: pair.privateKeyOpenssh, publicKey: pub)), encoding: .utf8)!
            return SecretItem(id: id, label: "", payload: json)
        }
        XCTAssertTrue(VaultSecretItems.accepts(key(pair.publicKeyOpenssh), knownProfiles: [], hasDefaultKey: false))
        let other = try generateSshKeypair()
        XCTAssertFalse(VaultSecretItems.accepts(key(other.publicKeyOpenssh), knownProfiles: [], hasDefaultKey: false))
        XCTAssertFalse(VaultSecretItems.accepts(key(pair.publicKeyOpenssh, id: "k_not-a-uuid"), knownProfiles: [], hasDefaultKey: false))
        XCTAssertTrue(VaultSecretItems.accepts(key(pair.publicKeyOpenssh, id: "k_" + UUID().uuidString), knownProfiles: [], hasDefaultKey: false))
    }
}

@MainActor
final class VaultSyncMigrationTests: XCTestCase {
    func testTheUpdateTurnsSyncOffAndNoticesWhoUsedIt() {
        let d = UserDefaults(suiteName: "vault-mig-\(UUID().uuidString)")!
        d.set(true, forKey: VaultSync.enabledKey)
        d.set(Data("x".utf8), forKey: "vault_state_profiles")
        let sync = VaultSync(defaults: d)
        let store = MemoryStoreForMigration()
        sync.store = store
        sync.migrateIfNeeded()
        XCTAssertFalse(sync.enabled)
        XCTAssertTrue(d.bool(forKey: VaultSync.noticeKey))
        d.set(true, forKey: VaultSync.enabledKey)
        sync.migrateIfNeeded()
        XCTAssertTrue(sync.enabled, "it runs once")
    }

    func testAFreshInstallHasSyncOffAndNoNotice() {
        let d = UserDefaults(suiteName: "vault-mig-\(UUID().uuidString)")!
        let sync = VaultSync(defaults: d)
        sync.store = MemoryStoreForMigration()
        sync.migrateIfNeeded()
        XCTAssertFalse(sync.enabled)
        XCTAssertFalse(d.bool(forKey: VaultSync.noticeKey))
    }
}

private final class MemoryStoreForMigration: VaultItemStore {
    var items: [String: Data] = [:]
    func load(_ account: String) -> Data? { items[account] }
    func save(_ data: Data, _ account: String) -> Bool { items[account] = data; return true }
    func delete(_ account: String) { items[account] = nil }
}
