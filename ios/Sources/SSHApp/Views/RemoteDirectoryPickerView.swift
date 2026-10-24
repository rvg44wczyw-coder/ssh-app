import SwiftUI
import SshCoreBridge

/// Folder chooser over the host's file listing (directories only). Paths under the host's home
/// are returned as `~/...` so a profile keeps working if the home folder is renamed.
struct RemoteDirectoryPickerView: View {
    let title: String
    @State var showHidden: Bool
    let start: String
    let list: (String?, Bool) async throws -> RemoteDirectoryListing
    let onPick: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var listing: RemoteDirectoryListing?
    @State private var home: String?
    @State private var error: String?
    @State private var loading = false

    init(title: String, showHidden: Bool, start: String, list: @escaping (String?, Bool) async throws -> RemoteDirectoryListing, onPick: @escaping (String) -> Void) {
        self.title = title
        _showHidden = State(initialValue: showHidden)
        self.start = start
        self.list = list
        self.onPick = onPick
    }

    private func shown(_ path: String) -> String {
        if let h = home, path == h { return "~" }
        if let h = home, path.hasPrefix(h + "/") { return "~" + path.dropFirst(h.count) }
        return path
    }

    private func load(_ path: String?) async {
        loading = true
        error = nil
        do {
            listing = try await list(path, showHidden)
        } catch {
            self.error = error.localizedDescription
        }
        loading = false
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let l = listing {
                    HStack {
                        Text(shown(l.currentPath))
                            .font(.system(size: 12, design: .monospaced)).lineLimit(1).truncationMode(.head)
                        Spacer()
                        Button("Use this folder") { onPick(shown(l.currentPath)); dismiss() }
                            .font(.caption.bold()).buttonStyle(.glassProminent).controlSize(.small)
                    }
                    .padding(.horizontal, 12).padding(.vertical, 8)
                }
                if let e = error { Text(e).font(.caption).foregroundColor(.red).padding(8) }
                List {
                    if let parent = listing?.parentPath {
                        Button { Task { await load(parent) } } label: { Label("..", systemImage: "arrow.up.left") }
                    }
                    ForEach((listing?.entries ?? []).filter { $0.kind == .directory }, id: \.path) { entry in
                        Button { Task { await load(entry.path) } } label: {
                            Label(entry.name, systemImage: "folder.fill").font(.system(size: 14, design: .monospaced))
                        }
                    }
                }
                .listStyle(.plain)
                .overlay { if loading { ProgressView() } }
            }
            .navigationTitle(title)
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItemGroup(placement: .primaryAction) {
                    Button { Task { await load("~") } } label: { Image(systemName: "house") }
                    Button { showHidden.toggle(); Task { await load(listing?.currentPath) } } label: {
                        Image(systemName: showHidden ? "eye" : "eye.slash")
                    }
                }
            }
            .task {
                if let h = try? await list("~", false) { home = h.currentPath }
                let s = start.trimmingCharacters(in: .whitespaces)
                await load(s.isEmpty ? "~" : s)
                if error != nil { await load("~") }
            }
        }
    }
}
