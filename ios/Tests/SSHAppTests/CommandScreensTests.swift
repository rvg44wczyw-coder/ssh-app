import XCTest
@testable import SSHApp

final class CommandScreensTests: XCTestCase {
    func testMovingTheCursorSendsOnlyArrows() {
        XCTAssertEqual(DialogKeys.move(from: 2, to: 2).count, 0)
        XCTAssertEqual(DialogKeys.move(from: 2, to: 5), Array(repeating: DialogKeys.down, count: 3))
        XCTAssertEqual(DialogKeys.move(from: 4, to: 1), Array(repeating: DialogKeys.up, count: 3))
    }
}
