import SwiftUI
import SshCoreBridge

public struct FileBrowserSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var showHidden: Bool = false
    @State private var searchText: String = ""
    @State private var showPreview: Bool = false

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

    private func icon(for kind: RemoteFileKind) -> String {
        switch kind {
        case .directory: return "folder.fill"
        case .symlink: return "link"
        case .file: return "doc.text"
        }
    }

    private func sizeText(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let listing = viewModel.fileListing {
                    HStack(spacing: 8) {
                        Text(listing.currentPath)
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
                                    .lineLimit(1)
                                Spacer()
                                if entry.kind == .file {
                                    Text(sizeText(entry.sizeBytes))
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                }
                            }
                        }
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
            .onAppear { if viewModel.fileListing == nil { load(nil) } }
        }
    }
}

struct FilePreviewView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    @Environment(\.dismiss) private var dismiss

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
                                    Text(line.isEmpty ? " " : line)
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
