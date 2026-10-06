import XCTest
@testable import SshCoreBridge

final class SshCoreBridgeTests: XCTestCase {
    func testVersion() {
        let version = coreVersion()
        XCTAssertEqual(version, "0.1.0")
    }

    func testKeypairGeneration() throws {
        let keys = try generateSshKeypair()
        XCTAssertTrue(keys.publicKeyOpenssh.hasPrefix("ssh-ed25519 "))
        XCTAssertTrue(keys.privateKeyOpenssh.contains("BEGIN OPENSSH PRIVATE KEY"))
    }

    func testApprovalFunctions() throws {
        let hash = hashCommandSha256(command: "git push origin main")
        XCTAssertEqual(hash, "16f880284c51ff513ff5465f0082c75d9c7ebb186e65e98b4fa362534044846a")

        let bytes = createCanonicalSigningBytes(
            id: "appr-1",
            commandHashSha256: hash,
            nonce: "testnonce",
            timestampSec: 1728000000,
            approved: true
        )
        let str = String(data: bytes, encoding: .utf8)
        XCTAssertEqual(str, "SSH_APP_APPROVAL_V1:appr-1:16f880284c51ff513ff5465f0082c75d9c7ebb186e65e98b4fa362534044846a:testnonce:1728000000:true")

        let isFresh = verifyApprovalFreshness(timestampSec: 1000, currentTimeSec: 1050, maxDriftSec: 120)
        XCTAssertTrue(isFresh)
    }
}
