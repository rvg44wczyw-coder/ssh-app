import Foundation

/// Remote path ↔ the text inside a File Provider item identifier (`p:` + base64url). A name only:
/// the core cleans and checks the path again on every use.
public enum ItemPath {
    public static func encode(_ path: String) -> String {
        Data(path.utf8).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    public static func decode(_ text: String) -> String? {
        var b64 = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while b64.count % 4 != 0 { b64 += "=" }
        guard let data = Data(base64Encoded: b64), let s = String(data: data, encoding: .utf8),
              s.hasPrefix("/") else { return nil }
        return s
    }

    public static func parent(of path: String) -> String {
        guard let slash = path.lastIndex(of: "/"), slash != path.startIndex else { return "/" }
        return String(path[..<slash])
    }

    /// `parent` + `/` + `name`; the name must be a single plain component.
    public static func child(_ parent: String, _ name: String) -> String? {
        guard !name.isEmpty, name != ".", name != "..", !name.contains("/"),
              !name.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7f }) else { return nil }
        return (parent == "/" ? "" : parent) + "/" + name
    }
}
