import XCTest
import SshCoreBridge
@testable import SSHApp

final class PairingTests: XCTestCase {
    private let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"

    func testTheDeviceNameKeepsOnlyWhatTheHostAccepts() {
        XCTAssertEqual(PairHostSheet.sanitizedDeviceName("Ivan's iPhone 15"), "Ivans iPhone 15")
        XCTAssertEqual(PairHostSheet.sanitizedDeviceName("Телефон"), "iPhone")
        XCTAssertEqual(PairHostSheet.sanitizedDeviceName("a\nb; rm"), "ab rm")
        XCTAssertEqual(PairHostSheet.sanitizedDeviceName(String(repeating: "x", count: 100)).count, 40)
        XCTAssertEqual(PairHostSheet.sanitizedDeviceName("  "), "iPhone")
    }

    func testTheNameTheSheetOffersIsAcceptedByTheCore() throws {
        let command = try pairingPasteCommand(publicKey: key, deviceName: PairHostSheet.defaultDeviceName())
        XCTAssertTrue(command.hasPrefix("sh -c '"))
        XCTAssertThrowsError(try pairingPasteCommand(publicKey: key, deviceName: "a\nb"))
        XCTAssertThrowsError(try pairingPasteCommand(publicKey: "junk", deviceName: "P"))
    }

    func testFindingsComeFromTheFacts() {
        var d = HostDoctor(
            helperVersion: "0.1.0", os: "macos", arch: "aarch64",
            tmux: DoctorTool(installed: false, version: ""), git: DoctorTool(installed: true, version: "2"),
            python3: DoctorTool(installed: true, version: ""), tailscale: DoctorTool(installed: true, version: ""),
            tailscaleIp: "", claudeFound: false, claudeSettingsExists: false, antigravityFound: false,
            antigravityHooksFileExists: false, ollamaInstalled: false, ollamaRunning: false,
            approvalKeyTrusted: true, hookScriptInstalled: false, helperDirPrivate: true)
        XCTAssertEqual(onboardingFindings(doctor: d).map(\.id), ["install-tmux"])
        XCTAssertEqual(onboardingFindings(doctor: d).first?.level, .needed)
        d.tmux = DoctorTool(installed: true, version: "3.4")
        XCTAssertTrue(onboardingFindings(doctor: d).isEmpty)
    }
}
