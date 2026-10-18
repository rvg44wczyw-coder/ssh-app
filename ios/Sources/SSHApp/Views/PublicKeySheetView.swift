import SwiftUI
import SshCoreBridge

/// The public key to add to `~/.ssh/authorized_keys`: the active server's key first, then the others.
struct PublicKeySheetView: View {
    let serverName: String?
    let activeKeyId: String?
    let onClose: () -> Void
    @ObservedObject private var keys = SSHKeyStore.shared
    @State private var copied: String? = nil

    private var rows: [(id: String?, name: String)] {
        let all: [(id: String?, name: String)] =
            [(nil, "Default key")] + keys.extra.map { (Optional($0.id), $0.name) }
        return all.sorted { ($0.id == activeKeyId ? 0 : 1) < ($1.id == activeKeyId ? 0 : 1) }
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(rows, id: \.id) { row in
                    let pub = KeychainManager.shared.getOrDerivePublicKey(id: row.id)
                    Section(header: Text(row.id == activeKeyId ? "\(row.name) — used by \(serverName ?? "this server")" : row.name)) {
                        if let pub, !pub.isEmpty {
                            Text(pub).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                            Button(copied == (row.id ?? "default") ? "Copied" : "Copy public key") {
                                KeychainManager.shared.copyPublicKeyToClipboard(pub)
                                copied = row.id ?? "default"
                            }
                        } else {
                            Text(row.id == nil ? "No default key yet: create one in Settings." : "Key is missing from the Keychain.")
                                .font(.caption).foregroundColor(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("Public key")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
        }
    }
}
