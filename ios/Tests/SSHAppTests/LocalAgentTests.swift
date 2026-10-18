import XCTest
import SshCoreBridge
@testable import SSHApp

final class LocalAgentTests: XCTestCase {
    func testEveryLocalToolMakesAProfileThatRoundTrips() throws {
        for tool in localAgentTools() {
            let p = try XCTUnwrap(SavedAgentProfile(localTool: tool.id, model: tool.takesModel ? "gemma4:26b" : ""), tool.id)
            XCTAssertTrue(p.isLocal)
            XCTAssertEqual(p.agent, tool.agent)
            XCTAssertEqual(p.supportsHooks, tool.hasApprovals, tool.id)
            let data = try JSONEncoder().encode(p)
            let back = try JSONDecoder().decode(SavedAgentProfile.self, from: data)
            XCTAssertEqual(back, p)
            if case .failure(let e) = back.launchCommand() { XCTFail("\(tool.id): \(e)") }
        }
    }

    func testTheModelOfALocalProfileIsInTheLaunchCommand() throws {
        var p = try XCTUnwrap(SavedAgentProfile(localTool: "opencode"))
        p.localModel = "qwen2.5-coder:7b"
        let cmd = try p.launchCommand().get()
        XCTAssertTrue(cmd.contains("launch"), cmd)
        XCTAssertTrue(cmd.contains("qwen2.5-coder:7b"), cmd)
        p.localModel = "; rm -rf ~"
        XCTAssertThrowsError(try p.launchCommand().get(), "a hostile model name must be refused, not quoted into a command")
    }

    func testProfilesSavedBeforeLocalToolsStillDecode() throws {
        let old = #"{"id":"x","name":"n","agentRaw":"claudeCode","binary":"claude","env":[],"args":[],"workingDir":""}"#
        let p = try JSONDecoder().decode(SavedAgentProfile.self, from: Data(old.utf8))
        XCTAssertFalse(p.isLocal)
        XCTAssertEqual(p.agent, .claudeCode)
    }

    func testNewAgentKindsKeepTheirOwnRawValues() {
        for kind in [AgentKind.openCode, .qwenCode, .crush, .cline, .aider, .goose, .ollamaChat, .claudeCode, .grok] {
            XCTAssertEqual(SavedAgentProfile.kind(SavedAgentProfile.raw(kind)), kind)
        }
    }

    func testOnlyToolsWithAHookOfferApprovalEvents() throws {
        for tool in localAgentTools() {
            let p = try XCTUnwrap(SavedAgentProfile(localTool: tool.id))
            XCTAssertEqual(p.approvalEventChoices.isEmpty, !tool.hasApprovals, tool.id)
            if tool.hasApprovals {
                XCTAssertNoThrow(try planAgentHooks(agent: tool.agent, preset: p.hookPreset, hookDir: nil), tool.id)
            }
        }
    }
}
