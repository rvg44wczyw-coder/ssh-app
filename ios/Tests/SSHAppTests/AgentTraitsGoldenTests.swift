import XCTest
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
            XCTAssertEqual(AgentTraits.resolve(agentType: type, detectedAgy: agy, detectedClaude: claude).id, want,
                           "\(type) agy=\(agy) claude=\(claude)")
        }
    }
}
