import XCTest
@testable import SSHApp

final class TerminalFolderTests: XCTestCase {
    func testAcceptedAndRejectedFolders() {
        XCTAssertEqual(TerminalFolder.normalized(" ~/Terminals "), "~/Terminals")
        XCTAssertEqual(TerminalFolder.normalized("/srv/work"), "/srv/work")
        XCTAssertEqual(TerminalFolder.normalized("~"), "~")
        XCTAssertNil(TerminalFolder.normalized(""))
        XCTAssertNil(TerminalFolder.normalized("relative/dir"))
        XCTAssertNil(TerminalFolder.normalized("~other/x"))
        XCTAssertNil(TerminalFolder.normalized("/a\nb"))
        XCTAssertNil(TerminalFolder.normalized("/" + String(repeating: "x", count: 500)))
    }

    func testProblemTextOnlyForNonEmptyBadValues() {
        XCTAssertNil(TerminalFolder.problem(""))
        XCTAssertNil(TerminalFolder.problem("~/ok"))
        XCTAssertNotNil(TerminalFolder.problem("nope"))
    }
}
