import XCTest
import SshCoreBridge
@testable import SSHApp

final class BackupTests: XCTestCase {
    private func server(_ name: String, fingerprint: String? = nil) -> ServerProfile {
        ServerProfile(name: name, host: "mac.example.com", username: "user", hostKeyFingerprint: fingerprint)
    }

    private func build(_ selection: BackupService.Selection, servers: [ServerProfile] = [], profiles: [SavedAgentProfile] = []) -> [BackupGroup] {
        BackupService.buildGroups(servers: servers, profiles: profiles, snippets: [], configJSON: "{}", keys: [], selection: selection)
    }

    func testServersRoundTripThroughTheCoreFile() throws {
        let s = server("Laptop", fingerprint: "SHA256:abc")
        var sel = BackupService.Selection()
        sel.fingerprints = true
        let groups = build(sel, servers: [s])
        let file = try backupExport(groups: groups, passphrase: nil, createdAtMs: 1, appVersion: "t")
        XCTAssertFalse(backupIsEncrypted(file: file))
        let back = try backupOpen(file: file, passphrase: nil)
        let rows = BackupService.servers(in: try XCTUnwrap(back.groups.first))
        XCTAssertEqual(rows.map(\.name), ["Laptop"])
        XCTAssertEqual(rows.first?.fingerprint, "SHA256:abc")
    }

    func testFingerprintsAreLeftOutByDefault() {
        let groups = build(BackupService.Selection(), servers: [server("Laptop", fingerprint: "SHA256:abc")])
        XCTAssertNil(BackupService.servers(in: groups[0]).first?.fingerprint)
    }

    func testSecretGroupsNeverGoIntoAPlainFile() throws {
        let g = BackupGroup(id: BackupService.GroupID.keys, secret: true, json: "[]")
        XCTAssertThrowsError(try backupExport(groups: [g], passphrase: nil, createdAtMs: 1, appVersion: "t"))
    }

    func testEncryptedFileNeedsThePassphrase() throws {
        let g = BackupGroup(id: BackupService.GroupID.secretValues, secret: true, json: "[]")
        let pass = Data("correct horse battery staple".utf8)
        let file = try backupExport(groups: [g], passphrase: pass, createdAtMs: 1, appVersion: "t")
        XCTAssertTrue(backupIsEncrypted(file: file))
        XCTAssertThrowsError(try backupOpen(file: file, passphrase: Data("wrong wrong wrong wrong".utf8)))
        XCTAssertEqual(try backupOpen(file: file, passphrase: pass).groups.count, 1)
    }

    func testHostileRecordsAreDropped() {
        let bad = BackupGroup(id: BackupService.GroupID.servers, secret: false, json: """
        [{"id":"not-a-uuid","name":"x","host":"h","port":22,"username":"u","isDefault":false},
         {"id":"\(UUID().uuidString)","name":"ok","host":"h","port":22,"username":"u","isDefault":false},
         {"id":"\(UUID().uuidString)","name":"-oProxyCommand=x","host":"-oProxyCommand=evil","port":22,"username":"u","isDefault":false}]
        """)
        XCTAssertEqual(BackupService.servers(in: bad).map(\.name), ["ok"])
    }

    func testMalformedGroupJSONGivesNothing() {
        let g = BackupGroup(id: BackupService.GroupID.snippets, secret: false, json: "{not json")
        XCTAssertTrue(BackupService.snippets(in: g).isEmpty)
        XCTAssertTrue(BackupService.keys(in: g).isEmpty)
        XCTAssertTrue(BackupService.secretValues(in: g).isEmpty)
    }

    @MainActor
    func testRestoreNeverDeletesAndMergeKeepsExisting() {
        let m = ServerProfilesManager()
        let before = m.profiles
        let a = server("Alpha")
        _ = m.restore([a], overwrite: false)
        var changed = a
        changed.name = "Changed"
        _ = m.restore([changed], overwrite: false)
        XCTAssertEqual(m.profiles.first { $0.id == a.id }?.name, "Alpha")
        _ = m.restore([changed], overwrite: true)
        XCTAssertEqual(m.profiles.first { $0.id == a.id }?.name, "Changed")
        XCTAssertGreaterThanOrEqual(m.profiles.count, before.count + 1)
        XCTAssertFalse(m.profiles.contains { $0.isDefault && $0.id == a.id && !before.isEmpty })
    }
}

final class AppLockTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_000_000)

    func testDisabledNeverLocks() {
        XCTAssertFalse(AppLock.shouldLock(enabled: false, backgroundedAt: nil, now: now, after: 0))
    }

    func testColdStartLocks() {
        XCTAssertTrue(AppLock.shouldLock(enabled: true, backgroundedAt: nil, now: now, after: 900))
    }

    func testGraceWindow() {
        let t = now.addingTimeInterval(-59)
        XCTAssertFalse(AppLock.shouldLock(enabled: true, backgroundedAt: t, now: now, after: 60))
        XCTAssertTrue(AppLock.shouldLock(enabled: true, backgroundedAt: now.addingTimeInterval(-60), now: now, after: 60))
        XCTAssertTrue(AppLock.shouldLock(enabled: true, backgroundedAt: now, now: now, after: 0))
    }
}

final class BackupSandboxHostsTests: XCTestCase {
    func testSandboxHostsTravelInPlainGroupAndHostileRecordsAreDropped() throws {
        let d = UserDefaults(suiteName: "backup-sbx-\(UUID().uuidString)")!
        SandboxAllowStore.set(["registry.npmjs.org"], for: "codex", defaults: d)
        let items = SyncedSandboxHost.outgoing(defaults: d)
        let groups = BackupService.buildGroups(servers: [], profiles: [], snippets: [], configJSON: "", keys: [],
                                               selection: BackupService.Selection(), sandboxHosts: items)
        let g = try XCTUnwrap(groups.first { $0.id == BackupService.GroupID.sandboxHosts })
        XCTAssertFalse(g.secret)
        XCTAssertEqual(BackupService.sandboxHosts(in: g).map(\.host), ["registry.npmjs.org"])
        let forged = BackupGroup(id: g.id, secret: false,
                                 json: #"[{"id":"s_codex_evil.com; id","payload":"{\"agent\":\"codex\",\"host\":\"evil.com; id\"}"}]"#)
        XCTAssertTrue(BackupService.sandboxHosts(in: forged).isEmpty)
    }
}
