import XCTest
import SshCoreBridge
@testable import SSHApp

final class HookProfileTests: XCTestCase {
    func testApplyingATemplateFillsTheProfileHookSettings() throws {
        var profile = SavedAgentProfile(name: "p", agent: .claudeCode, binary: "claude")
        let template = try XCTUnwrap(hookTemplatesFor(agent: .claudeCode).first { $0.id == "block-rm-rf" })
        profile.apply(template: template.preset)
        XCTAssertEqual(profile.hookKindRaw, "custom")
        XCTAssertEqual(profile.customHookEvents, ["PreToolUse"])
        XCTAssertEqual(profile.customHookMatcher, "Bash")
        XCTAssertFalse(profile.customHookCommand.isEmpty)
        XCTAssertNoThrow(try planAgentHooks(agent: .claudeCode, preset: profile.hookPreset, hookDir: nil))

        let notify = try XCTUnwrap(hookTemplatesFor(agent: .claudeCode).first { $0.id == "notify-finished" })
        profile.apply(template: notify.preset)
        XCTAssertEqual(profile.hookKindRaw, "notifyOnly")
        XCTAssertEqual(profile.approvalEvents, ["Stop"])
    }

    func testEveryTemplatePlansForItsAgents() {
        for agent in [AgentKind.claudeCode, .antigravity, .codex, .grok] {
            for t in hookTemplatesFor(agent: agent) {
                var p = SavedAgentProfile(name: "p", agent: agent, binary: "x")
                p.apply(template: t.preset)
                XCTAssertNoThrow(try planAgentHooks(agent: agent, preset: p.hookPreset, hookDir: nil), "\(t.id) / \(agent)")
            }
        }
    }

    func testExtraEventsOnlyForClaude() {
        XCTAssertTrue(SavedAgentProfile(name: "a", agent: .claudeCode, binary: "claude").extraApprovalEvents.contains("SessionStart"))
        XCTAssertTrue(SavedAgentProfile(name: "a", agent: .grok, binary: "grok").extraApprovalEvents.isEmpty)
    }

    func testDriftIsReportedOnlyWhenSomethingIsInstalled() throws {
        let planned = try planAgentHooks(agent: .claudeCode, preset: defaultHookPreset(), hookDir: nil)
        XCTAssertTrue(hooksDrift(planned: planned, installed: []).isEmpty)
        XCTAssertTrue(hooksDrift(planned: planned, installed: planned).isEmpty)
        XCTAssertFalse(hooksDrift(planned: planned, installed: Array(planned.dropFirst())).isEmpty)
    }
}
