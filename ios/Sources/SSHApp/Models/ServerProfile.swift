import Foundation

public struct ServerProfile: Identifiable, Codable, Equatable, Hashable, Sendable {
    public let id: UUID
    public var name: String
    public var host: String
    public var port: UInt16
    public var username: String
    public var isDefault: Bool
    /// SHA-256 fingerprint of the host key pinned on first connect (`SHA256:...`); a different key
    /// afterwards is refused by the core as a possible man-in-the-middle.
    public var hostKeyFingerprint: String?
    /// Which SSH key signs in to this server (`SSHKeyStore`); `nil` = the default key.
    public var keyId: String?

    public init(
        id: UUID = UUID(),
        name: String,
        host: String,
        port: UInt16 = 22,
        username: String,
        isDefault: Bool = false,
        hostKeyFingerprint: String? = nil,
        keyId: String? = nil
    ) {
        self.id = id
        self.name = name
        self.host = host
        self.port = port
        self.username = username
        self.isDefault = isDefault
        self.hostKeyFingerprint = hostKeyFingerprint
        self.keyId = keyId
    }
}
