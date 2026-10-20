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

    func testPresentationStyleRoundTripAndFallback() {
        for style in FeedPresentationStyle.allCases {
            XCTAssertEqual(FeedPresentationStyle.resolved(style.rawValue), style)
        }
        XCTAssertEqual(FeedPresentationStyle.resolved("nonsense"), FeedPresentationStyle.fallback)
        XCTAssertEqual(FeedPresentationStyle.resolved(""), .assistant)
    }

    func testGitContextLabel() {
        let ctx = GitSessionContext(isGit: true, repoName: "ssh-app", branch: "main", worktreeName: "ios-tests",
                                    isWorktree: true, detached: false)
        XCTAssertEqual(ctx.displayLabel, "ssh-app · main · wt ios-tests")
        XCTAssertNil(GitSessionContext.none.displayLabel)
    }

    func testLongGitNamesAreShortenedInTheMiddle() {
        XCTAssertEqual(GitSessionContext.short("main", limit: 28), "main")
        let long = "feature/very-long-branch-name-for-ticket-ABC-12345"
        let short = GitSessionContext.short(long, limit: GitSessionContext.branchLimit)
        XCTAssertEqual(short.count, GitSessionContext.branchLimit)
        XCTAssertTrue(short.hasPrefix("feature/"))
        XCTAssertTrue(short.hasSuffix("12345"))
        XCTAssertTrue(short.contains("…"))
        let ctx = GitSessionContext(isGit: true, repoName: "r", branch: long, worktreeName: nil, isWorktree: false, detached: false)
        XCTAssertEqual(ctx.label(full: true), "r · " + long)
        XCTAssertEqual(ctx.label(full: false), "r · " + short)
        // emoji / multi-scalar names are cut on characters, not bytes
        XCTAssertEqual(GitSessionContext.short(String(repeating: "é", count: 40), limit: 10).count, 10)
    }

    func testDraftAppendTypesOnlyTheNewEnd() {
        XCTAssertEqual(DraftSync.bytes(from: "hel", to: "hello"), Array("lo".utf8))
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "hello"), [])
    }

    func testDraftDeletionIsBackspaces() {
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "he"), [0x7f, 0x7f, 0x7f])
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: ""), Array(repeating: 0x7f, count: 5))
    }

    func testDraftEditInTheMiddleRetypesFromThere() {
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "heXlo"), [0x7f, 0x7f, 0x7f] + Array("Xlo".utf8))
    }

    func testDraftWithNewlineIsAPasteAndControlCharactersAreDropped() {
        let b = DraftSync.bytes(from: "a", to: "a\nb\u{3}\u{1b}[A")
        XCTAssertEqual(String(decoding: b ?? [], as: UTF8.self), "\u{1b}[200~\nb[A\u{1b}[201~")
    }

    func testUncertainDraftIsClearedGenerouslyAndRetyped() {
        let b = DraftSync.bytes(from: "ab", to: "abc", uncertain: true) ?? []
        XCTAssertEqual(b.prefix(24).filter { $0 == 0x7f }.count, 24)
        XCTAssertEqual(Array(b.suffix(3)), Array("abc".utf8))
    }

    func testDraftTooLongToDeleteIsNotMirrored() {
        XCTAssertNil(DraftSync.bytes(from: String(repeating: "x", count: 5000), to: ""))
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
