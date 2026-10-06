import Foundation
import SshCoreBridge

public struct AgentApprovalRequest: Identifiable, Equatable, Sendable, Codable {
    public let id: String
    public let agentName: String
    public let command: String
    public let cwd: String
    public let timestampSec: UInt64
    public let nonce: String
    public let commandHashSha256: String

    public init(
        id: String,
        agentName: String,
        command: String,
        cwd: String,
        timestampSec: UInt64? = nil,
        nonce: String? = nil
    ) {
        self.id = id
        self.agentName = agentName
        self.command = command
        self.cwd = cwd
        self.timestampSec = timestampSec ?? UInt64(Date().timeIntervalSince1970)
        self.nonce = nonce ?? UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
        self.commandHashSha256 = hashCommandSha256(command: command)
    }

    public var canonicalBytes: Data {
        createCanonicalSigningBytes(
            id: id,
            commandHashSha256: commandHashSha256,
            nonce: nonce,
            timestampSec: timestampSec,
            approved: true
        )
    }
}

public struct SignedApprovalResult: Equatable, Sendable, Codable {
    public let id: String
    public let approved: Bool
    public let signatureHex: String
    public let publicKeyOpenssh: String
    public let timestampSec: UInt64
}
