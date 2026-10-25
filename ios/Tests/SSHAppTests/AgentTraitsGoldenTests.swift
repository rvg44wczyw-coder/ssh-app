import XCTest
import SshCoreBridge
@testable import SSHApp

/// Phase 0 of the universal agent feed (docs/features/60-universal-agent-feed.md): the whole table of
/// `AgentTraits` and how a tab resolves to it, frozen. The refactor moves this table to the Rust core;
/// these expectations must hold for the descriptor read from there.
final class AgentTraitsGoldenTests: XCTestCase {
    func testEveryFieldOfEveryEntry() {
        let esc: [UInt8] = [0x1b, 0x5b, 0x41]
        XCTAssertEqual(AgentTraits.claude,
                       AgentTraits(id: "claude", sendNowKeys: [0x18, 0x13], pullQueueKeys: esc,
                                   clearLineKeys: [0x15], acceptSuggestionKeys: [0x09], imageReferencePrefix: "@"))
        XCTAssertEqual(AgentTraits.antigravity,
                       AgentTraits(id: "agy", sendNowKeys: [0x1b, 0x6d], pullQueueKeys: esc,
                                   clearLineKeys: [0x15], acceptSuggestionKeys: nil, imageReferencePrefix: ""))
        XCTAssertEqual(AgentTraits.generic,
                       AgentTraits(id: "generic", sendNowKeys: nil, pullQueueKeys: esc,
                                   clearLineKeys: [0x15], acceptSuggestionKeys: nil, imageReferencePrefix: ""))
    }

    /// Every tab type with every detection result: what the feed drives the agent with.
    func testResolveMatrix() {
        let cases: [(AgentType, Bool, Bool, String)] = [
            (.claudeCode, false, false, "claude"), (.claudeCode, true, false, "agy"),
            (.claudeCode, false, true, "claude"), (.claudeCode, true, true, "agy"),
            (.antigravity, false, false, "agy"), (.antigravity, true, false, "agy"),
            (.antigravity, false, true, "claude"), (.antigravity, true, true, "agy"),
            (.localLlm, false, false, "generic"), (.localLlm, true, false, "agy"),
            (.localLlm, false, true, "claude"), (.localLlm, true, true, "agy"),
            (.customShell, false, false, "generic"), (.customShell, true, false, "agy"),
            (.customShell, false, true, "claude"), (.customShell, true, true, "agy"),
            (.dockerContainer, false, false, "generic"), (.dockerContainer, true, false, "agy"),
            (.dockerContainer, false, true, "claude"), (.dockerContainer, true, true, "agy"),
        ]
        XCTAssertEqual(cases.count, AgentType.allCases.count * 4)
        for (type, agy, claude, want) in cases {
            let dialect: ScreenDialect = agy ? .agy : (claude ? .claude : .unknown)
            XCTAssertEqual(AgentResolver.descriptor(agentType: type, dialect: dialect).id, want,
                           "\(type) agy=\(agy) claude=\(claude)")
        }
    }

    /// The command menus the app has always offered, now read from the core's registry.
    func testCommandMenusAreThoseOfBefore() {
        let claude = AgentResolver.entry("claude").commands
        XCTAssertEqual(claude.map(\.name), ["/clear", "/compact", "/context", "/resume", "/memory", "/model",
                                            "/permissions", "/config", "/mcp", "/agents", "/hooks", "/status",
                                            "/cost", "/doctor", "/help"])
        XCTAssertEqual(claude.filter(\.dividerBefore).map(\.name), ["/model", "/status"])
        XCTAssertEqual(claude.filter(\.destructive).map(\.name), ["/clear"])
        let agy = AgentResolver.entry("agy").commands
        XCTAssertEqual(agy.map(\.name), ["/clear", "/context", "/resume", "/rewind", "/fork", "/rename", "/copy",
                                         "/diff", "/model", "/effort", "/permissions", "/config", "/mcp", "/agents",
                                         "/hooks", "/plugin", "/credits", "/changelog", "/help"])
        XCTAssertEqual(agy.filter(\.dividerBefore).map(\.name), ["/model", "/credits"])
        XCTAssertEqual(AgentResolver.entry("generic").commands.count, 0)
    }

    func testTheScreenWinsOverTheTabType() {
        XCTAssertEqual(AgentResolver.descriptor(agentType: .customShell, dialect: .agy).id, "agy")
        XCTAssertEqual(AgentResolver.descriptor(agentType: .antigravity, dialect: .claude).id, "claude")
        XCTAssertEqual(AgentResolver.descriptor(agentType: .claudeCode, dialect: .unknown).id, "claude")
        XCTAssertEqual(AgentResolver.descriptor(agentType: .customShell, dialect: .unknown).id, "generic")
        XCTAssertNil(AgentResolver.detected(dialect: .unknown))
        XCTAssertEqual(AgentResolver.detected(dialect: .agy)?.sessionStore, .database)
        XCTAssertEqual(AgentResolver.detected(dialect: .claude)?.sessionStore, .file)
    }

    func testADialectSwitchNeedsTwoLooksInARow() {
        var t = DialectTracker()
        XCTAssertEqual(t.observe(.claude), .claude)          // from nothing: at once
        XCTAssertEqual(t.observe(.agy), .claude)             // one look of another agent: held
        XCTAssertEqual(t.observe(.claude), .claude)          // back: the doubt is gone
        XCTAssertEqual(t.observe(.agy), .claude)
        XCTAssertEqual(t.observe(.agy), .agy)                // twice in a row: switched
        XCTAssertEqual(t.observe(.unknown), .unknown)        // the agent left: at once
        XCTAssertEqual(t.observe(.claude), .claude)
    }
}
