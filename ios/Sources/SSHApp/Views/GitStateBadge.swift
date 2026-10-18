import SshCoreBridge
import SwiftUI

/// Colours and letters of the git states shown in the file list.
enum GitStatePresentation {
    static func color(_ s: GitFileState) -> Color {
        switch s {
        case .added, .untracked: return .green
        case .modified: return .orange
        case .deleted: return .red
        case .renamed: return .blue
        case .conflicted: return .purple
        }
    }

    static func letter(_ s: GitFileState) -> String {
        switch s {
        case .added, .untracked: return "N"
        case .modified: return "M"
        case .deleted: return "D"
        case .renamed: return "R"
        case .conflicted: return "!"
        }
    }

    /// The state a folder shows: conflicts first, then deletions, edits, renames and new files.
    static func strongest(_ states: [GitFileState]) -> GitFileState? {
        for s in [GitFileState.conflicted, .deleted, .modified, .renamed, .added, .untracked] where states.contains(s) {
            return s
        }
        return nil
    }
}

/// The `N` / `M` / `D` mark at the right of a file row; a folder shows a dot.
struct GitStateBadge: View {
    let state: GitFileState
    var folder = false

    var body: some View {
        if folder {
            Circle().fill(GitStatePresentation.color(state)).frame(width: 8, height: 8)
        } else {
            Text(GitStatePresentation.letter(state))
                .font(.system(size: 11, weight: .bold, design: .monospaced))
                .foregroundColor(GitStatePresentation.color(state))
                .frame(width: 18)
        }
    }
}
