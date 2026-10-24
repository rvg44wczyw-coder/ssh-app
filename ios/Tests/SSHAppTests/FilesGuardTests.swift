import XCTest
@testable import SSHApp

@MainActor
final class FilesGuardTests: XCTestCase {
    private func server(_ id: String = "A", guarded: Bool = false) -> SharedServer {
        SharedServer(id: id, name: "Mac", host: "h", port: 22, username: "u", keyId: nil, hostKey: "SHA256:x", startFolder: nil, guarded: guarded)
    }

    func testAListFromAnOlderAppIsNotGuarded() throws {
        let old = #"[{"id":"A","name":"Mac","host":"h","port":22,"username":"u","hostKey":"SHA256:x"}]"#
        let list = try JSONDecoder().decode([SharedServer].self, from: Data(old.utf8))
        XCTAssertEqual(list.first?.guarded, false)
    }

    func testGuardedSurvivesAnEncodeDecodeRound() throws {
        let data = try JSONEncoder().encode([server(guarded: true)])
        XCTAssertEqual(try JSONDecoder().decode([SharedServer].self, from: data).first?.guarded, true)
    }

    /// A changed switch makes the published server different, so the extension drops its open
    /// connection and the next request asks again.
    func testSwitchChangesTheServerIdentity() {
        XCTAssertNotEqual(server(guarded: false), server(guarded: true))
    }

    func testPublishedServersFollowTheSwitch() {
        let defaults = UserDefaults.standard
        let before = defaults.object(forKey: FilesIntegration.requireAuthKey)
        defer { if let before { defaults.set(before, forKey: FilesIntegration.requireAuthKey) } else { defaults.removeObject(forKey: FilesIntegration.requireAuthKey) } }
        let p = ServerProfile(name: "Mac", host: "h", username: "u", hostKeyFingerprint: "SHA256:x")
        defaults.set(false, forKey: FilesIntegration.requireAuthKey)
        XCTAssertEqual(FilesIntegration.sharedServers(from: [p]).map(\.guarded), [false])
        defaults.set(true, forKey: FilesIntegration.requireAuthKey)
        XCTAssertEqual(FilesIntegration.sharedServers(from: [p]).map(\.guarded), [true])
    }
}
