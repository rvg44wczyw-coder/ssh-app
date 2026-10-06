import SwiftUI
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#endif

public struct DiffViewerSheetView: View {
    @Environment(\.dismiss) private var dismiss
    public let serverProfile: ServerProfile?
    public let privateKeyOpenssh: String?

    @State private var diffText: String = ""
    @State private var isLoading: Bool = true
    @State private var errorMessage: String? = nil

    public init(serverProfile: ServerProfile?, privateKeyOpenssh: String?) {
        self.serverProfile = serverProfile
        self.privateKeyOpenssh = privateKeyOpenssh
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if isLoading {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text("Fetching git diff from host...")
                            .font(.system(size: 13))
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let error = errorMessage {
                    VStack(spacing: 12) {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.system(size: 32))
                            .foregroundColor(.orange)
                        Text(error)
                            .font(.system(size: 13))
                            .foregroundColor(.secondary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal)
                        Button("Retry") {
                            fetchDiff()
                        }
                        .buttonStyle(.bordered)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if diffText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    VStack(spacing: 12) {
                        Image(systemName: "checkmark.circle")
                            .font(.system(size: 36))
                            .foregroundColor(.green)
                        Text("Working tree clean")
                            .font(.system(size: 14, weight: .semibold))
                        Text("No uncommitted changes found in repository.")
                            .font(.system(size: 12))
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView([.horizontal, .vertical]) {
                        LazyVStack(alignment: .leading, spacing: 1) {
                            ForEach(Array(diffText.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                                diffLineView(line)
                            }
                        }
                        .padding(8)
                    }
                    .background(Color.black)
                }
            }
            .navigationTitle("Git Diff")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button(action: fetchDiff) {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(isLoading)
                }
            }
            .onAppear {
                fetchDiff()
            }
        }
    }

    private func diffLineView(_ line: String) -> some View {
        let (bgColor, fgColor) = lineColors(for: line)
        return Text(line.isEmpty ? " " : line)
            .font(.system(size: 11, design: .monospaced))
            .foregroundColor(fgColor)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(bgColor)
    }

    private func lineColors(for line: String) -> (Color, Color) {
        if line.hasPrefix("+++") || line.hasPrefix("---") {
            return (Color.gray.opacity(0.2), .primary)
        } else if line.hasPrefix("+") {
            return (Color.green.opacity(0.18), .green)
        } else if line.hasPrefix("-") {
            return (Color.red.opacity(0.18), .red)
        } else if line.hasPrefix("@@") {
            return (Color.cyan.opacity(0.15), .cyan)
        } else if line.hasPrefix("diff --git") {
            return (Color.yellow.opacity(0.2), .yellow)
        } else {
            return (Color.clear, .primary)
        }
    }

    private func fetchDiff() {
        guard let server = serverProfile, let privKey = privateKeyOpenssh, !privKey.isEmpty else {
            self.errorMessage = "No active server profile or private key available."
            self.isLoading = false
            return
        }

        self.isLoading = true
        self.errorMessage = nil

        let config = RemoteServerConfig(
            host: server.host,
            port: server.port,
            username: server.username,
            privateKeyPem: privKey
        )

        Task {
            do {
                let output = try await executeRemoteCommand(config: config, command: "git diff HEAD 2>&1")
                await MainActor.run {
                    self.diffText = output
                    self.isLoading = false
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = "Failed to run git diff: \(error.localizedDescription)"
                    self.isLoading = false
                }
            }
        }
    }
}
