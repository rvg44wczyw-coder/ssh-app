import XCTest
@testable import SSHApp

final class PromptSubmitTests: XCTestCase {
    func testBoxStillHoldingTheSentMessageIsDetected() {
        XCTAssertTrue(BoxCheck.stillHolds(box: "fix the login bug please", sent: "fix the login bug please"))
        XCTAssertTrue(BoxCheck.stillHolds(box: "  fix the login\n bug please ", sent: "fix the login bug please"))
    }

    func testAWrappedBoxStillMatches() {
        let sent = "line one\nline two with more words in it"
        XCTAssertTrue(BoxCheck.stillHolds(box: "line one line two with more words in it", sent: sent))
    }

    func testAnEmptyOrUnreadableBoxIsGone() {
        XCTAssertFalse(BoxCheck.stillHolds(box: "", sent: "hello there"))
        XCTAssertFalse(BoxCheck.stillHolds(box: nil, sent: "hello there"))
    }

    func testSomethingElseInTheBoxIsLeftAlone() {
        // A hint, or text typed at the laptop: not ours to press Enter on or to clear.
        XCTAssertFalse(BoxCheck.stillHolds(box: "Press up to edit queued messages", sent: "hello there"))
        XCTAssertFalse(BoxCheck.stillHolds(box: "something typed at the laptop", sent: "hello there"))
    }

    func testAnEmptySentTextNeverMatches() {
        XCTAssertFalse(BoxCheck.stillHolds(box: "anything", sent: "  \n "))
    }
}
