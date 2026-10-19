import XCTest
@testable import SSHApp

final class AgentFeedTests: XCTestCase {
    func testSingleLineIsTextAndEnter() {
        XCTAssertEqual(FeedSubmit.bytes(for: "  hello \n"), Array("hello".utf8) + [0x0d])
    }

    func testMultiLineIsOneBracketedPaste() {
        let b = FeedSubmit.bytes(for: "a\nb")
        XCTAssertEqual(b, Array("\u{1b}[200~a\nb\u{1b}[201~".utf8) + [0x0d])
    }

    func testPasteMarkersInsideTheTextAreRemoved() {
        let b = FeedSubmit.bytes(for: "x\u{1b}[201~\nrm -rf ~")
        let text = String(decoding: b, as: UTF8.self)
        XCTAssertEqual(text.components(separatedBy: "\u{1b}[201~").count, 2)
    }

    func testMarkdownSplitsFencedCode() {
        let segs = FeedMarkdown.segments("intro\n```swift\nlet a = 1\n```\nafter")
        XCTAssertEqual(segs, [.prose("intro"), .code(language: "swift", code: "let a = 1"), .prose("after")])
    }

    func testUnclosedFenceRunsToTheEnd() {
        let segs = FeedMarkdown.segments("```py\nprint(1)")
        XCTAssertEqual(segs, [.code(language: "py", code: "print(1)")])
    }
}

final class TerminalPasteTests: XCTestCase {
    func testBracketedPasteKeepsLinesTogether() {
        let b = TerminalPaste.bytes("a\r\nb", bracketed: true)
        XCTAssertEqual(String(decoding: b, as: UTF8.self), "\u{1b}[200~a\nb\u{1b}[201~")
    }

    func testPlainPasteTurnsLineBreaksIntoEnter() {
        XCTAssertEqual(TerminalPaste.bytes("a\nb", bracketed: false), Array("a\rb".utf8))
    }

    func testEndMarkerInsideThePasteIsRemoved() {
        let text = String(decoding: TerminalPaste.bytes("x\u{1b}[201~y", bracketed: true), as: UTF8.self)
        XCTAssertEqual(text, "\u{1b}[200~xy\u{1b}[201~")
    }
}
