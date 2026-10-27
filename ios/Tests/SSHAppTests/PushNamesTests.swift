import XCTest
@testable import SSHApp

final class PushNamesTests: XCTestCase {
    func testAKnownSidPutsTheNameIntoTheText() {
        XCTAssertEqual(PushNames.body(sid: "0a1b2c3d", text: "Waiting for you", names: ["0a1b2c3d": .init(name: "agy")]),
                       "agy: Waiting for you")
    }

    func testAnUnknownOrMissingSidKeepsTheRelaysText() {
        XCTAssertEqual(PushNames.body(sid: "ffffffff", text: "Waiting for you", names: ["0a1b2c3d": .init(name: "agy")]), "Waiting for you")
        XCTAssertEqual(PushNames.body(sid: nil, text: "Waiting for you", names: [:]), "Waiting for you")
    }

    func testABlockedBodyNamesTheFirstHostAndCountsTheRest() {
        XCTAssertEqual(PushNames.blockedBody(name: "Sandbox x", hosts: ["a.com"], text: "t"), "Sandbox x: blocked a.com")
        XCTAssertEqual(PushNames.blockedBody(name: "Sandbox x", hosts: ["a.com", "b.com", "c.com"], text: "t"),
                       "Sandbox x: blocked a.com and 2 more")
        XCTAssertEqual(PushNames.blockedBody(name: "Sandbox x", hosts: [], text: "A sandbox blocked a host"),
                       "Sandbox x: A sandbox blocked a host")
    }

    func testANameIsFlattenedAndCut() {
        XCTAssertEqual(PushNames.clean("a\nb\tc"), "a b c")
        XCTAssertEqual(PushNames.clean(String(repeating: "x", count: 100)).count, PushNames.maxLength)
    }

    func testAServerListFromAnOlderAppAllowsTheBannerLookupAndTheSwitchSurvivesEncoding() throws {
        let old = #"{"id":"1","name":"n","host":"h","port":22,"username":"u","hostKey":"SHA256:x"}"#
        XCTAssertTrue(try JSONDecoder().decode(SharedServer.self, from: Data(old.utf8)).bannerLookup)
        let off = SharedServer(id: "1", name: "n", host: "h", port: 22, username: "u", keyId: nil,
                               hostKey: "SHA256:x", startFolder: nil, bannerLookup: false)
        let back = try JSONDecoder().decode(SharedServer.self, from: JSONEncoder().encode(off))
        XCTAssertFalse(back.bannerLookup)
    }
}
