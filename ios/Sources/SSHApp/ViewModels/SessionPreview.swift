import Foundation
import SwiftTerm

/// Renders the tail of a tab's output through a headless terminal (same emulator as the real
/// view, so cursor movements and redraws of programs like Claude Code come out as the screen
/// looked) and returns the last non-empty lines for a card preview.
@MainActor
final class SessionPreviewCache {
    static let shared = SessionPreviewCache()
    private var cache: [UUID: (length: Int, lines: [String])] = [:]

    func lines(for vm: TerminalSessionViewModel, maxLines: Int) -> [String] {
        let data = vm.getHistoryBuffer()
        if let hit = cache[vm.session.id], hit.length == data.count { return hit.lines }
        let lines = Self.render(data, size: vm.terminalSize, maxLines: maxLines)
        cache[vm.session.id] = (data.count, lines)
        return lines
    }

    func forget(_ id: UUID) { cache[id] = nil }

    private final class Delegate: TerminalDelegate {
        func send(source: Terminal, data: ArraySlice<UInt8>) {}
        func showCursor(source: Terminal) {}
        func hideCursor(source: Terminal) {}
        func setTerminalTitle(source: Terminal, title: String) {}
        func setTerminalIconTitle(source: Terminal, title: String) {}
        func sizeChanged(source: Terminal) {}
        func windowCommand(source: Terminal, command: Terminal.WindowManipulationCommand) -> [UInt8]? { nil }
        func scrolled(source: Terminal, yDisp: Int) {}
        func linefeed(source: Terminal) {}
        func bufferActivated(source: Terminal) {}
        func bell(source: Terminal) {}
    }

    private static func render(_ data: Data, size: (cols: Int, rows: Int), maxLines: Int) -> [String] {
        guard !data.isEmpty else { return [] }
        // Only the tail matters; 128 KB is more than a few screens of output.
        let tail = data.count > 131_072 ? data.suffix(131_072) : data
        let delegate = Delegate()
        let terminal = Terminal(delegate: delegate, options: TerminalOptions(cols: max(size.cols, 20), rows: max(size.rows, 8), scrollback: 200))
        terminal.feed(byteArray: [UInt8](tail))
        let text = String(decoding: terminal.getBufferAsData(), as: UTF8.self)
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            .map { String($0) }
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        return Array(lines.suffix(maxLines))
    }
}
