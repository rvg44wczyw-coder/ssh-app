import XCTest

/// Spec 60: a new agent is a row in the core's registry, not a new branch in the feed code. The
/// feed files must not name an agent: ids, hook names and program names come from the descriptor
/// (`AgentResolver`). The tab's own type (`AgentType.claudeCode`, ...) is part of the tab model,
/// not an agent identity, and stays (the compiler checks those switches for completeness).
final class FeedHasNoAgentLiteralsTests: XCTestCase {
    private static let feedFiles = [
        "Views/ActiveTerminalSessionView.swift",
        "Views/StructuredTranscriptView.swift",
        "Views/SessionUsageStrip.swift",
        "Views/TmuxSessionsSheetView.swift",
        "Views/MainTerminalView.swift",
        "ViewModels/TerminalSessionViewModel.swift"
    ]

    private static let forbidden = ["\"claude\"", "\"agy\"", "\"antigravity\"", "\"codex\"", "\"grok\"", "isAgy", "isClaude", "screenIsAgy", "screenIsClaude"]

    func testTheFeedFilesNameNoAgent() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/SSHApp")
        var hits: [String] = []
        for file in Self.feedFiles {
            let text = try String(contentsOf: root.appendingPathComponent(file), encoding: .utf8)
            for (n, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                let code = line.trimmingCharacters(in: .whitespaces)
                if code.hasPrefix("//") { continue }
                for bad in Self.forbidden where line.contains(bad) { hits.append("\(file):\(n + 1) \(bad)") }
            }
        }
        XCTAssertTrue(hits.isEmpty, "agent literals in the feed code: \(hits)")
    }
}
