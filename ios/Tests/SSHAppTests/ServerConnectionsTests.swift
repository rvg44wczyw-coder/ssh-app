import XCTest
import SshCoreBridge
@testable import SSHApp

@MainActor
final class ServerConnectionsTests: XCTestCase {
    func testAServerWithoutAPinnedHostKeyCannotBeOpened() {
        let p = ServerProfile(name: "Mac", host: "h", username: "u")
        XCTAssertFalse(ServerConnections.canOpen(p))
        XCTAssertFalse(ServerConnections.canOpen(nil))
        var empty = p
        empty.hostKeyFingerprint = ""
        XCTAssertFalse(ServerConnections.canOpen(empty))
    }

    func testAServerModelWithoutAPinRefusesToConnect() async {
        let vm = TerminalSessionViewModel(session: TabSession(title: "Mac"))
        await vm.connectForServer(host: "127.0.0.1", port: 1, username: "u", keyRef: KeyRef(id: nil, guarded: false))
        guard case .failed(let reason) = vm.state else { return XCTFail("state \(vm.state)") }
        XCTAssertTrue(reason.contains("not pinned"), reason)
        XCTAssertNil(vm.sessionHandle)
    }

    func testClosingWithNothingOpenIsHarmless() {
        ServerConnections.shared.closeAll()
        ServerConnections.shared.release(UUID())
    }
}
