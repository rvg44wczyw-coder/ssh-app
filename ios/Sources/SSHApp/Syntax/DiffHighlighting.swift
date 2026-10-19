import SwiftUI

/// Syntax colours for the code inside a unified diff. Every file's added/context lines and its
/// removed/context lines are highlighted as two small documents (so strings and comments that span
/// lines come out right) and mapped back to the diff line they came from.
enum DiffHighlighter {
    private static let maxFiles = 30

    /// Diff line index -> the code of that line (without the +/-/space marker) with colours.
    static func highlight(diff: String, dark: Bool) async -> [Int: AttributedString] {
        guard SyntaxSettings.enabled else { return [:] }
        struct Side { var text: [String] = []; var diffIndex: [Int] = [] }
        struct FileParts { var name: String; var old = Side(); var new = Side() }
        var files: [FileParts] = []
        var inHunk = false
        for (i, line) in diff.components(separatedBy: "\n").enumerated() {
            if line.hasPrefix("diff --git ") {
                inHunk = false
                // "diff --git a/path b/path": the new name is after the last " b/".
                let name = line.range(of: " b/", options: .backwards).map { String(line[$0.upperBound...]) } ?? ""
                if files.count < maxFiles { files.append(FileParts(name: name)) } else { break }
                continue
            }
            if line.hasPrefix("@@") { inHunk = true; continue }
            guard inHunk, !files.isEmpty, let first = line.first else { continue }
            let code = String(line.dropFirst())
            let k = files.count - 1
            switch first {
            case "+":
                files[k].new.text.append(code); files[k].new.diffIndex.append(i)
            case "-":
                files[k].old.text.append(code); files[k].old.diffIndex.append(i)
            case " ":
                files[k].new.text.append(code); files[k].new.diffIndex.append(i)
                files[k].old.text.append(code); files[k].old.diffIndex.append(-1)
            default: break
            }
        }
        var out: [Int: AttributedString] = [:]
        for f in files where !f.name.isEmpty {
            for side in [f.new, f.old] where !side.text.isEmpty {
                guard let lines = await SyntaxService.shared.highlight(
                    text: side.text.joined(separator: "\n"), filename: f.name, dark: dark),
                    lines.count == side.text.count
                else { continue }
                for (n, line) in lines.enumerated() where side.diffIndex[n] >= 0 {
                    if out[side.diffIndex[n]] == nil { out[side.diffIndex[n]] = line.attributed() }
                }
            }
        }
        return out
    }
}
