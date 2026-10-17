import XCTest
@testable import SSHApp

@MainActor
final class FilesIntegrationTests: XCTestCase {
    func testPathRoundTripAndRejectedIdentifiers() {
        for p in ["/", "/Users/me/Terminals", "/a b/it's/ü/日本語", "/x/../y"] {
            XCTAssertEqual(ItemPath.decode(ItemPath.encode(p)), p)
        }
        XCTAssertNil(ItemPath.decode("not base64 !!"))
        XCTAssertNil(ItemPath.decode(ItemPath.encode("relative/path")), "only absolute paths")
        XCTAssertNil(ItemPath.decode(""))
        XCTAssertFalse(ItemPath.encode("/a?b>c").contains("/"))
    }

    func testParentAndChild() {
        XCTAssertEqual(ItemPath.parent(of: "/a/b"), "/a")
        XCTAssertEqual(ItemPath.parent(of: "/a"), "/")
        XCTAssertEqual(ItemPath.parent(of: "/"), "/")
        XCTAssertEqual(ItemPath.child("/", "x"), "/x")
        XCTAssertEqual(ItemPath.child("/a", "x y"), "/a/x y")
        for bad in ["", ".", "..", "a/b", "a\nb", "a\u{7}"] {
            XCTAssertNil(ItemPath.child("/a", bad), bad)
        }
    }

    func testOnlyPinnedAndEnabledServersAreShared() {
        let pinned = ServerProfile(name: "A", host: "a", username: "u", hostKeyFingerprint: "SHA256:x")
        let unpinned = ServerProfile(name: "B", host: "b", username: "u")
        let off = ServerProfile(name: "C", host: "c", username: "u", hostKeyFingerprint: "SHA256:y", showInFiles: false)
        let empty = ServerProfile(name: "D", host: "d", username: "u", hostKeyFingerprint: "")
        let shared = FilesIntegration.sharedServers(from: [pinned, unpinned, off, empty])
        XCTAssertEqual(shared.map(\.name), ["A"])
        XCTAssertEqual(shared.first?.hostKey, "SHA256:x")
        XCTAssertNil(shared.first?.startFolder)
    }

    func testFolderIsTrimmedAndEmptyMeansHome() {
        let a = ServerProfile(name: "A", host: "a", username: "u", hostKeyFingerprint: "SHA256:x", filesFolder: "  /srv/files ")
        let b = ServerProfile(name: "B", host: "b", username: "u", hostKeyFingerprint: "SHA256:x", filesFolder: "   ")
        let shared = FilesIntegration.sharedServers(from: [a, b])
        XCTAssertEqual(shared[0].startFolder, "/srv/files")
        XCTAssertNil(shared[1].startFolder)
    }

    func testOldSavedServersStillDecode() throws {
        let json = #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","name":"A","host":"h","port":22,"username":"u","isDefault":true}"#
        let p = try JSONDecoder().decode(ServerProfile.self, from: Data(json.utf8))
        XCTAssertNil(p.showInFiles)
        XCTAssertNil(p.filesFolder)
    }

    func testSharedServerRoundTrip() throws {
        let s = SharedServer(id: "1", name: "n", host: "h", port: 2222, username: "u", keyId: nil, hostKey: "SHA256:k", startFolder: "/x")
        XCTAssertEqual(try JSONDecoder().decode(SharedServer.self, from: JSONEncoder().encode(s)), s)
    }
}
