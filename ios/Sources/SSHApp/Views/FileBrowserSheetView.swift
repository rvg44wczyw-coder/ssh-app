import SwiftUI
import SshCoreBridge

public struct FileBrowserSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var showHidden: Bool = false
    @State private var searchText: String = ""
    @State private var showPreview: Bool = false
    @State private var showDiff: Bool = false
    @State private var projectTab: ProjectTab?

    public init(viewModel: TerminalSessionViewModel, onClose: (() -> Void)? = nil) {
        self.viewModel = viewModel
        self.onClose = onClose
    }

    private func closeSheet() {
        onClose?()
        dismiss()
    }

    private func load(_ path: String?) {
        viewModel.loadRemoteDirectory(path: path, includeHidden: showHidden)
    }

    private var filteredEntries: [RemoteFileEntry] {
        guard let entries = viewModel.fileListing?.entries else { return [] }
        let q = searchText.trimmingCharacters(in: .whitespaces)
        if q.isEmpty { return entries }
        return entries.filter { $0.name.localizedCaseInsensitiveContains(q) }
    }

    /// Git state per absolute path, and per folder the strongest state inside it.
    private var fileStates: [String: GitFileState] {
        Dictionary(viewModel.gitStatus?.changes.map { ($0.path, $0.state) } ?? [], uniquingKeysWith: { a, _ in a })
    }

    private func folderState(_ dir: String) -> GitFileState? {
        let prefix = dir + "/"
        let inside = (viewModel.gitStatus?.changes ?? []).filter { $0.path.hasPrefix(prefix) }.map(\.state)
        return GitStatePresentation.strongest(inside)
    }

    /// Files git reports as deleted that belonged to the listed folder (they are gone from `ls`).
    private var deletedHere: [String] {
        guard let dir = viewModel.fileListing?.currentPath else { return [] }
        let q = searchText.trimmingCharacters(in: .whitespaces)
        return (viewModel.gitStatus?.changes ?? []).compactMap { c in
            guard c.state == .deleted, c.path.hasPrefix(dir + "/") else { return nil }
            let name = String(c.path.dropFirst(dir.count + 1))
            guard !name.contains("/"), q.isEmpty || name.localizedCaseInsensitiveContains(q) else { return nil }
            return name
        }.sorted()
    }

    private func icon(for kind: RemoteFileKind) -> String {
        switch kind {
        case .directory: return "folder.fill"
        case .symlink: return "link"
        case .file: return "doc.text"
        }
    }

    private func nameColor(_ entry: RemoteFileEntry) -> Color {
        let state = entry.kind == .directory ? folderState(entry.path) : fileStates[entry.path]
        return state.map(GitStatePresentation.color) ?? .primary
    }

    private func sizeText(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let listing = viewModel.fileListing {
                    HStack(spacing: 8) {
                        if let branch = viewModel.gitStatus?.branch {
                            Button { projectTab = .branches } label: {
                                Label(branch, systemImage: "arrow.triangle.branch")
                                    .font(.caption.bold())
                                    .foregroundColor(.accentColor)
                                    .lineLimit(1)
                            }
                            .buttonStyle(.plain)
                            .accessibilityHint("Open the branch list")
                        }
                        Text(displaySafeText(raw: listing.currentPath))
                            .font(.system(size: 12, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.head)
                        Spacer()
                        Button("cd Here") {
                            viewModel.changeDirectory(to: listing.currentPath)
                            closeSheet()
                        }
                        .font(.caption.bold())
                        .buttonStyle(.bordered)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                }

                if let error = viewModel.fileBrowserError {
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundColor(.yellow)
                        Text(error).font(.caption)
                        Spacer()
                    }
                    .padding(10)
                    .background(Color.red.opacity(0.2))
                }

                List {
                    if let parent = viewModel.fileListing?.parentPath {
                        Button(action: { load(parent) }) {
                            Label("..", systemImage: "arrow.up.left")
                        }
                    }
                    ForEach(deletedHere, id: \.self) { name in
                        HStack {
                            Image(systemName: "doc.text").foregroundColor(GitStatePresentation.color(.deleted))
                            Text(name)
                                .font(.system(size: 14, design: .monospaced))
                                .strikethrough()
                                .foregroundColor(GitStatePresentation.color(.deleted))
                                .lineLimit(1)
                            Spacer()
                            GitStateBadge(state: .deleted)
                        }
                    }
                    if viewModel.fileListing?.isTruncated == true {
                        Label("More entries exist than are shown", systemImage: "ellipsis.circle")
                            .font(.caption)
                            .foregroundColor(.orange)
                    }
                    ForEach(filteredEntries, id: \.path) { entry in
                        Button(action: {
                            if entry.kind == .directory {
                                searchText = ""
                                load(entry.path)
                            } else {
                                viewModel.loadRemoteFilePreview(path: entry.path)
                                showPreview = true
                            }
                        }) {
                            HStack {
                                Image(systemName: icon(for: entry.kind))
                                    .foregroundColor(entry.kind == .directory ? .yellow : .secondary)
                                Text(entry.name)
                                    .font(.system(size: 14, design: .monospaced))
                                    .foregroundColor(nameColor(entry))
                                    .lineLimit(1)
                                Spacer()
                                if let s = entry.kind == .directory ? folderState(entry.path) : fileStates[entry.path] {
                                    GitStateBadge(state: s, folder: entry.kind == .directory)
                                }
                                if entry.isRestricted {
                                    Image(systemName: "lock.fill").font(.caption2).foregroundColor(.secondary)
                                }
                                if entry.kind == .file {
                                    Text(sizeText(entry.sizeBytes))
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                }
                            }
                        }
                        .disabled(entry.isRestricted)
                        .opacity(entry.isRestricted ? 0.5 : 1)
                        .contextMenu {
                            Button("Insert Path") { viewModel.insertRemotePath(entry.path) }
                        }
                    }
                }
                .listStyle(.plain)
                .overlay {
                    if viewModel.isFileBrowserLoading { ProgressView() }
                }
            }
            .searchable(text: $searchText, prompt: "Filter")
            .navigationTitle("Files")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { closeSheet() }
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    if let git = viewModel.gitStatus {
                        Menu {
                            Button { showDiff = true } label: { Label("Uncommitted changes (diff)", systemImage: "plusminus.circle") }
                            Button { projectTab = .changes } label: { Label("Stage, commit, push", systemImage: "square.and.arrow.up") }
                            Button { projectTab = .log } label: { Label("Log and commit actions", systemImage: "clock.arrow.circlepath") }
                            Button { projectTab = .branches } label: { Label("Branches, worktrees, stashes", systemImage: "arrow.triangle.branch") }
                        } label: {
                            Image(systemName: "arrow.triangle.branch")
                                .overlay(alignment: .topTrailing) {
                                    if !git.changes.isEmpty {
                                        Text("\(git.changes.count)\(git.truncated ? "+" : "")")
                                            .font(.system(size: 9, weight: .bold))
                                            .padding(.horizontal, 3)
                                            .background(Capsule().fill(Color.orange))
                                            .foregroundColor(.black)
                                            .offset(x: 8, y: -6)
                                    }
                                }
                        }
                        .accessibilityLabel("Git")
                    }
                    Button(action: { load("~") }) { Image(systemName: "house") }
                        .accessibilityLabel("Home")
                    Button(action: {
                        showHidden.toggle()
                        load(viewModel.fileListing?.currentPath)
                    }) {
                        Image(systemName: showHidden ? "eye" : "eye.slash")
                    }
                    .accessibilityLabel("Toggle Hidden Files")
                }
            }
            .sheet(isPresented: $showPreview) {
                FilePreviewView(viewModel: viewModel)
            }
            .sheet(isPresented: $showDiff) {
                DiffViewerSheetView(viewModel: viewModel)
            }
            .sheet(item: $projectTab) { tab in
                ProjectSheetView(viewModel: viewModel, tab: tab) { load(viewModel.fileListing?.currentPath) }
            }
            .onAppear { if viewModel.fileListing == nil { load(nil) } }
        }
    }
}

struct FilePreviewView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var scheme
    @AppStorage(SyntaxSettings.enabledKey) private var syntaxOn = true
    @AppStorage(SyntaxSettings.themeKey) private var syntaxTheme = SyntaxTheme.auto.rawValue
    @AppStorage(SyntaxSettings.maxKBKey) private var syntaxMaxKB = 200
    @State private var highlighted: [HighlightedLine]?

    var body: some View {
        NavigationStack {
            Group {
                if let preview = viewModel.filePreview {
                    ScrollView([.vertical, .horizontal]) {
                        let lines = preview.content.components(separatedBy: "\n")
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(lines.prefix(500).enumerated()), id: \.offset) { idx, line in
                                HStack(alignment: .top, spacing: 8) {
                                    Text("\(idx + 1)")
                                        .foregroundColor(.secondary)
                                        .frame(width: 36, alignment: .trailing)
                                    if let h = highlighted, idx < h.count, !line.isEmpty {
                                        Text(h[idx].attributed())
                                    } else {
                                        Text(line.isEmpty ? " " : line)
                                    }
                                }
                                .font(.system(size: 12, design: .monospaced))
                            }
                            if preview.isTruncated || lines.count > 500 {
                                Text("… truncated")
                                    .font(.caption)
                                    .foregroundColor(.orange)
                                    .padding(.top, 8)
                            }
                        }
                        .padding(8)
                    }
                } else if let error = viewModel.fileBrowserError {
                    Text(error).font(.caption).padding()
                } else {
                    ProgressView()
                }
            }
            .task(id: "\(viewModel.filePreview?.path ?? "")-\(syntaxOn)-\(syntaxTheme)-\(syntaxMaxKB)-\(scheme == .dark)") {
                highlighted = nil
                guard syntaxOn, let preview = viewModel.filePreview, !preview.isBinary else { return }
                highlighted = await SyntaxService.shared.highlight(
                    text: preview.content, filename: preview.filename, dark: scheme == .dark)
            }
            .navigationTitle(viewModel.filePreview?.filename ?? "Preview")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    if let preview = viewModel.filePreview, !preview.isBinary {
                        Button("Copy") {
                            #if canImport(UIKit)
                            UIPasteboard.general.string = preview.content
                            #endif
                        }
                        Button("Insert Path") {
                            viewModel.insertRemotePath(preview.path)
                            dismiss()
                        }
                    }
                }
            }
        }
        .onDisappear { viewModel.filePreview = nil }
    }
}
