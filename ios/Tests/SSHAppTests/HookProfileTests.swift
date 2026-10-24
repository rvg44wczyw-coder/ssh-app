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
        for agent in [AgentKind.claudeCode, .antigravity, .codex, .grok, .openCode, .qwenCode, .crush] {
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

    func testExtrasAndOwnCommandsSurviveStorageAndPlan() throws {
        var p = SavedAgentProfile(name: "p", agent: .claudeCode, binary: "claude")
        p.hookExtras = [SavedHookExtra(event: "PermissionRequest", command: "logger x", runsAfter: false, stopOnFailure: true, timeoutSecs: 5)]
        p.hookOwn = [SavedHookOwn(event: "SessionStart", command: "echo hi")]
        let back = try JSONDecoder().decode(SavedAgentProfile.self, from: JSONEncoder().encode(p))
        XCTAssertEqual(back.hookExtras, p.hookExtras)
        XCTAssertEqual(back.hookOwn, p.hookOwn)
        XCTAssertTrue(hookPresetUsesExtra(preset: back.hookPreset))
        let plan = try planAgentHooks(agent: .claudeCode, preset: back.hookPreset, hookDir: nil)
        XCTAssertTrue(plan.contains { $0.event == "SessionStart" })
    }

    func testOldProfilesWithoutTheNewFieldsStillLoad() throws {
        let p = SavedAgentProfile(name: "p", agent: .claudeCode, binary: "claude")
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(p)) as? [String: Any])
        json["hookExtras"] = nil
        json["hookOwn"] = nil
        let back = try JSONDecoder().decode(SavedAgentProfile.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertTrue(back.hookExtras.isEmpty && back.hookOwn.isEmpty)
    }

    func testProfilesWithExtraCommandsStayOffTheCloud() {
        var p = SavedAgentProfile(name: "p", agent: .claudeCode, binary: "claude")
        XCTAssertEqual(SyncedProfile.outgoing([p]).count, 1)
        p.hookExtras = [SavedHookExtra(event: "Stop", command: "x")]
        XCTAssertTrue(SyncedProfile.outgoing([p]).isEmpty)
    }
}

final class ApprovalTextTests: XCTestCase {
    func testEscapedLettersBecomeReadable() {
        XCTAssertEqual(ApprovalText.readable("echo \\u041f\\u0440\\u0438\\u0432\\u0435\\u0442"), "echo Привет")
        XCTAssertEqual(ApprovalText.readable("\\ud83d\\ude00"), "😀")
    }

    func testDangerousOrBrokenEscapesStayEscaped() {
        for raw in ["\\u001b[31m", "\\u202eevil", "\\u200b", "\\ud83d", "\\u00", "plain", "\\u0041"] {
            XCTAssertEqual(ApprovalText.readable(raw), raw, raw)
        }
    }
}
