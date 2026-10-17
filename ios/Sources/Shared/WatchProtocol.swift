import Foundation

/// What the iPhone shows the Apple Watch and what the watch may answer. Feature 22.
/// The watch gets a sanitised, shortened copy of a pending approval and can only send back
/// `{id, verdict}`; the phone re-reads the request from the host and signs the host's own copy.
public struct WatchApproval: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var agent: String
    public var tool: String
    /// Last part of the command (the end of a long command must not hide below the fold).
    public var tail: String
    /// Last folder name of the working directory.
    public var dir: String
    public var flags: [String]
    /// Seconds left on the host's clock when the phone published this.
    public var expiresIn: Int
    public var server: String

    public init(id: String, agent: String, tool: String, tail: String, dir: String,
                flags: [String], expiresIn: Int, server: String) {
        self.id = id
        self.agent = agent
        self.tool = tool
        self.tail = tail
        self.dir = dir
        self.flags = flags
        self.expiresIn = expiresIn
        self.server = server
    }
}

public struct WatchContext: Codable, Equatable, Sendable {
    public var v: Int
    public var seq: Int
    public var items: [WatchApproval]

    public init(v: Int = WatchProtocol.version, seq: Int, items: [WatchApproval]) {
        self.v = v
        self.seq = seq
        self.items = items
    }
}

public enum WatchReply: String, Sendable {
    case signed, expired, notPending = "not_pending", hostUnreachable = "host_unreachable", rejected
}

public enum WatchProtocol {
    public static let version = 1
    public static let maxItems = 5
    public static let maxTail = 160

    /// A request id as the host makes it: 32 lowercase hex characters.
    public static func isValidId(_ s: String) -> Bool {
        s.utf8.count == 32 && s.utf8.allSatisfy { ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x61 && $0 <= 0x66) }
    }

    /// Host-supplied text for a tiny screen: no control or bidirectional-formatting characters, capped
    /// (keeping the end when `keepEnd`).
    public static func sanitize(_ text: String, max: Int, keepEnd: Bool = false) -> String {
        let bad: Set<UInt32> = [0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066, 0x2067, 0x2068, 0x2069]
        let cleaned = String(String.UnicodeScalarView(text.unicodeScalars.filter {
            if $0 == "\n" || $0 == "\t" { return true }
            return $0.value >= 0x20 && $0.value != 0x7f && !($0.value >= 0x80 && $0.value < 0xa0) && !bad.contains($0.value)
        }))
        guard cleaned.count > max else { return cleaned }
        return keepEnd ? "…" + String(cleaned.suffix(max)) : String(cleaned.prefix(max)) + "…"
    }

    // MARK: Transport dictionaries (WatchConnectivity only carries property-list values)

    public static func encode(_ context: WatchContext) -> [String: Any] {
        guard let data = try? JSONEncoder().encode(context) else { return [:] }
        return ["json": data]
    }

    public static func decode(_ dict: [String: Any]) -> WatchContext? {
        guard let data = dict["json"] as? Data,
              let ctx = try? JSONDecoder().decode(WatchContext.self, from: data),
              ctx.v == version, ctx.items.count <= maxItems,
              ctx.items.allSatisfy({ isValidId($0.id) }) else { return nil }
        return ctx
    }

    public static func verdictMessage(id: String, approved: Bool, seq: Int) -> [String: Any] {
        ["v": version, "id": id, "verdict": approved ? "allow" : "deny", "seq": seq]
    }

    /// Validates a message from the watch; anything unexpected is dropped.
    public static func parseVerdict(_ message: [String: Any]) -> (id: String, approved: Bool, seq: Int)? {
        guard (message["v"] as? Int) == version,
              let id = message["id"] as? String, isValidId(id),
              let verdict = message["verdict"] as? String, verdict == "allow" || verdict == "deny",
              let seq = message["seq"] as? Int else { return nil }
        return (id, verdict == "allow", seq)
    }

    public static func replyMessage(_ reply: WatchReply) -> [String: Any] {
        ["ok": reply == .signed, "state": reply.rawValue]
    }
}
