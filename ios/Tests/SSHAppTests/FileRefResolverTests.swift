import XCTest
import SshCoreBridge
@testable import SSHApp

final class FileRefResolverTests: XCTestCase {
    private func call(_ json: String) -> TranscriptToolCall {
        TranscriptToolCall(callId: UUID().uuidString, toolName: "Read", argumentsSummary: "", fullArgumentsJson: json,
                           result: nil, isError: false, status: .done)
    }

    private func turn(_ calls: [TranscriptToolCall]) -> TranscriptTurn {
        TranscriptTurn(id: UUID().uuidString, stepIndex: 0, turnType: .assistant, timestamp: nil, content: nil, thinking: nil,
                       toolCalls: calls, status: .done, label: nil)
    }

    func testTouchedPathsComeFromToolCallsNewestFirst() {
        let turns = [
            turn([call(#"{"file_path":"/p/old.swift"}"#)]),
            turn([call(#"{"path":"/p/ios/Views/Feed.swift"}"#), call(#"{"file_path":"relative.swift"}"#)]),
            turn([call("not json")]),
        ]
        XCTAssertEqual(FileRefResolver.touchedPaths(in: turns), ["/p/ios/Views/Feed.swift", "/p/old.swift"])
    }

    func testMatchesByTrailingComponents() {
        let known = ["/p/ios/Views/Feed.swift", "/p/other/Feed.swift", "/p/ios/Models/Feed.swift", "/p/Feed.swift.bak"]
        XCTAssertEqual(FileRefResolver.matches("Feed.swift", in: known),
                       ["/p/ios/Views/Feed.swift", "/p/other/Feed.swift", "/p/ios/Models/Feed.swift"])
        XCTAssertEqual(FileRefResolver.matches("Views/Feed.swift", in: known), ["/p/ios/Views/Feed.swift"])
        XCTAssertEqual(FileRefResolver.matches("./ios/Views/Feed.swift", in: known), ["/p/ios/Views/Feed.swift"])
        XCTAssertTrue(FileRefResolver.matches("eed.swift", in: known).isEmpty)
    }

    func testOnlyPlainRelativeMentionsAreSearched() {
        for ok in ["Feed.swift", "ios/Views/Feed.swift", "a-b_c.d+e@2x.png"] { XCTAssertTrue(FileRefResolver.isSearchable(ok), ok) }
        for bad in ["", "-rf.sh", "/etc/passwd", "~/x.sh", "../x.sh", "a/../../x.sh", "a b.sh", "x';rm -rf.sh", "$(id).sh", "a*.sh"] {
            XCTAssertFalse(FileRefResolver.isSearchable(bad), bad)
            XCTAssertNil(FileRefResolver.searchCommand(root: "/p", ref: bad), bad)
        }
        XCTAssertNil(FileRefResolver.searchCommand(root: "relative", ref: "Feed.swift"))
    }

    func testSearchCommandShapeAndParsing() throws {
        let cmd = try XCTUnwrap(FileRefResolver.searchCommand(root: "/p/it's here", ref: "deep/mod.rs"))
        // The root is one quoted word (the quote in its name is closed and reopened), the name is quoted too.
        XCTAssertTrue(cmd.hasPrefix("cd '/p/it'\\''s here' 2>/dev/null || exit 0;"), cmd)
        XCTAssertTrue(cmd.contains("git ls-files -- ':(glob)**/mod.rs' 'mod.rs'"), cmd)
        XCTAssertTrue(cmd.contains("-name 'node_modules'") && cmd.contains("-name 'mod.rs' -type f"), cmd)
        // Output lines are root-relative; the closest to the root come first, and a path that ends only
        // partly like the mention is dropped.
        let out = "src/deep/mod.rs\n./mod.rs\nsrc/x/other/deep/mod.rs\nsrc/shallow.rs\n"
        XCTAssertEqual(FileRefResolver.parseSearch(out, root: "/p", ref: "mod.rs"),
                       ["/p/mod.rs", "/p/src/deep/mod.rs", "/p/src/x/other/deep/mod.rs"].filter { $0.hasSuffix("mod.rs") })
        XCTAssertEqual(FileRefResolver.parseSearch(out, root: "/p/", ref: "deep/mod.rs"),
                       ["/p/src/deep/mod.rs", "/p/src/x/other/deep/mod.rs"])
    }
}
