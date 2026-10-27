import XCTest
import SshCoreBridge
@testable import SSHApp

final class BackgroundTasksTests: XCTestCase {
    private let rule = String(repeating: "─", count: 60)

    private func claudeScreen(_ mode: String) -> String {
        "⏺ done\n\n\(rule)\n❯ \n\(rule)\n  Sonnet 5.5\n  \(mode)\n"
    }

    func testTheChipCountComesFromTheCore() {
        XCTAssertEqual(parseScreenFeed(text: claudeScreen("⏵⏵ auto mode on · 3 shells")).background?.count, 3)
        XCTAssertNil(parseScreenFeed(text: claudeScreen("⏵⏵ auto mode on")).background)
    }

    func testOpenIsAllowedOnlyOnTheAgentsOwnQuietScreen() {
        let feed = parseScreenFeed(text: claudeScreen("⏵⏵ auto mode on · 1 shell"))
        XCTAssertTrue(backgroundOpenAllowed(feed: feed, expected: .claude))
        XCTAssertFalse(backgroundOpenAllowed(feed: feed, expected: .agy))
    }

    func testStopIsAllowedOnlyWhileATaskDetailIsOnScreen() {
        let detail = "⏺ x\n\n\(rule)\n  Shell details\n  Status:   running\n  Runtime:  1s\n  Command:  sleep 90\n  Output:\n  No output available\n  ← to go back · Esc/Enter/Space to close · x to stop\n"
        XCTAssertTrue(backgroundStopAllowed(feed: parseScreenFeed(text: detail)))
        XCTAssertFalse(backgroundStopAllowed(feed: parseScreenFeed(text: claudeScreen("⏵⏵ auto mode on · 1 shell"))))
    }

    func testOnlyClaudeAndAgyCarryTaskKeys() {
        for d in AgentResolver.all {
            let drives = d.id == "claude" || d.id == "agy"
            XCTAssertEqual(!d.keys.openTasksKeys.isEmpty, drives, d.id)
            XCTAssertEqual(d.keys.stopTaskKeys != nil, drives, d.id)
        }
        XCTAssertEqual(AgentResolver.entry("claude").keys.stopTaskKeys, Data("x".utf8))
        XCTAssertEqual(AgentResolver.entry("agy").keys.stopTaskKeys, Data("k".utf8))
    }
}
