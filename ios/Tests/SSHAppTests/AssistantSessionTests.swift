import XCTest
import SshCoreBridge
@testable import SSHApp

@MainActor
final class AssistantSessionTests: XCTestCase {
    func testStoreKeepsOneAssistantPerServerAndShowsCardsOnlyWithHistory() {
        let store = AssistantStore()
        let a = UUID(), b = UUID()
        let sa = store.session(for: a)
        XCTAssertTrue(sa === store.session(for: a))
        XCTAssertTrue(store.withHistory(in: [a, b]).isEmpty, "no card before first use")
        sa.chatMessages = [ChatMessage(role: "user", content: "hi"), ChatMessage(role: "assistant", content: "hello")]
        XCTAssertEqual(store.withHistory(in: [a, b]).map(\.serverId), [a])
        XCTAssertEqual(sa.preview, "hello")
        // Clearing one server's chat leaves the others.
        let sb = store.session(for: b)
        sb.chatMessages = [ChatMessage(role: "user", content: "x")]
        sa.clearAll()
        XCTAssertEqual(store.withHistory(in: [a, b]).map(\.serverId), [b])
    }

    func testSendWithoutConnectionKeepsTheQuestionAndShowsAnError() {
        let s = AssistantSession(serverId: UUID())
        s.sendChat("  ls  ", modelName: "m", handle: nil, connected: false)
        XCTAssertEqual(s.chatMessages.map(\.content), ["ls"])
        XCTAssertEqual(s.chatError, "Session is not connected")
        XCTAssertFalse(s.isChatLoading)
        s.startAiQuery(userPrompt: "x", modelName: "m", handle: nil, connected: false, recentLines: [])
        XCTAssertEqual(s.aiError, "Session is not connected")
        XCTAssertFalse(s.isAiLoading)
    }

    func testInterruptEndsARunningRequestVisiblyAndKeepsHistory() {
        let s = AssistantSession(serverId: UUID())
        s.chatMessages = [ChatMessage(role: "user", content: "q")]
        s.isChatLoading = true
        s.isAiLoading = true
        s.interrupt()
        XCTAssertFalse(s.isChatLoading)
        XCTAssertFalse(s.isAiLoading)
        XCTAssertNotNil(s.chatError)
        XCTAssertNotNil(s.aiError)
        XCTAssertEqual(s.chatMessages.count, 1)
        // Nothing running: interrupt must not invent an error.
        let idle = AssistantSession(serverId: UUID())
        idle.interrupt()
        XCTAssertNil(idle.chatError)
        XCTAssertNil(idle.aiError)
    }
}
