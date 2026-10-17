import XCTest
@testable import SSHApp

final class QuickCommandTests: XCTestCase {
    func testClearCommandsAskByDefault() {
        XCTAssertTrue(QuickCommand(label: "clear", text: "clear").confirm)
        XCTAssertTrue(QuickCommand(label: "/clear", text: " /clear ").confirm)
        XCTAssertFalse(QuickCommand(label: "/model", text: "/model").confirm)
    }

    func testExplicitConfirmOverridesDefault() {
        XCTAssertFalse(QuickCommand(label: "clear", text: "clear", confirm: false).confirm)
        XCTAssertTrue(QuickCommand(label: "x", text: "ls", confirm: true).confirm)
    }

    func testOldSavedCommandWithoutConfirmFieldGetsDefaultFromText() throws {
        let json = #"[{"label":"clear","text":"clear","run":true},{"label":"/help","text":"/help","run":true}]"#
        let list = try JSONDecoder().decode([QuickCommand].self, from: Data(json.utf8))
        XCTAssertTrue(list[0].confirm)
        XCTAssertFalse(list[1].confirm)
    }

    func testRoundTripKeepsConfirm() throws {
        let cmd = QuickCommand(label: "a", text: "ls", confirm: true)
        let back = try JSONDecoder().decode(QuickCommand.self, from: JSONEncoder().encode(cmd))
        XCTAssertEqual(back, cmd)
    }

    func testBytesAppendReturnOnlyWhenRun() {
        XCTAssertEqual(QuickCommand(label: "a", text: "ls").bytes, Array("ls\r".utf8))
        XCTAssertEqual(QuickCommand(label: "a", text: "ls", run: false).bytes, Array("ls".utf8))
    }

    func testRunlessClearStillBypassesNothing() {
        // A command typed without Return is harmless, but the flag itself stays what the user set.
        XCTAssertTrue(QuickCommand(label: "clear", text: "clear", run: false).confirm)
    }
}
