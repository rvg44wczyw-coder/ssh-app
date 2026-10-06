import SwiftUI
import SshCoreBridge
#if canImport(UIKit)
import UIKit
#endif

public struct ApprovalBannerView: View {
    public let request: AgentApprovalRequest
    public let onApprove: (AgentApprovalRequest) -> Void
    public let onDeny: (AgentApprovalRequest) -> Void

    @State private var isExpanded: Bool = false

    public init(
        request: AgentApprovalRequest,
        onApprove: @escaping (AgentApprovalRequest) -> Void,
        onDeny: @escaping (AgentApprovalRequest) -> Void
    ) {
        self.request = request
        self.onApprove = onApprove
        self.onDeny = onDeny
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "shield.lefthalf.filled.trianglebadge.exclamationmark")
                    .foregroundColor(.yellow)
                    .font(.system(size: 14, weight: .bold))

                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(request.agentName)
                            .font(.system(size: 11, weight: .bold))
                            .foregroundColor(.accentColor)
                        Text("requests approval")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(.primary)
                    }

                    Text(request.cwd)
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer()

                Button(action: {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        isExpanded.toggle()
                    }
                }) {
                    Image(systemName: isExpanded ? "chevron.up.circle.fill" : "chevron.down.circle")
                        .foregroundColor(.secondary)
                        .font(.system(size: 13))
                }
            }

            // Command Preview Box
            HStack {
                Text(request.command)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundColor(.green)
                    .lineLimit(isExpanded ? nil : 2)
                    .textSelection(.enabled)
                Spacer()
            }
            .padding(6)
            .background(Color.black.opacity(0.4))
            .cornerRadius(6)

            // Extended Details when expanded
            if isExpanded {
                VStack(alignment: .leading, spacing: 2) {
                    detailRow(title: "Hash", value: String(request.commandHashSha256.prefix(16)) + "...")
                    detailRow(title: "Nonce", value: String(request.nonce.prefix(12)) + "...")
                    detailRow(title: "Security", value: "Ed25519 Hardware-Signed")
                }
                .font(.system(size: 9, design: .monospaced))
                .foregroundColor(.secondary)
                .padding(.horizontal, 2)
            }

            // Interactive Action Buttons
            HStack(spacing: 10) {
                Button(action: {
                    triggerHaptic(success: false)
                    onDeny(request)
                }) {
                    HStack(spacing: 4) {
                        Image(systemName: "xmark")
                            .font(.system(size: 10, weight: .bold))
                        Text("Deny")
                            .font(.system(size: 12, weight: .bold))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .background(Color.red.opacity(0.2))
                    .foregroundColor(.red)
                    .cornerRadius(6)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.red.opacity(0.4), lineWidth: 1)
                    )
                }

                Button(action: {
                    triggerHaptic(success: true)
                    onApprove(request)
                }) {
                    HStack(spacing: 4) {
                        Image(systemName: "checkmark.shield.fill")
                            .font(.system(size: 11, weight: .bold))
                        Text("Approve (Sign)")
                            .font(.system(size: 12, weight: .bold))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .background(Color.green.opacity(0.25))
                    .foregroundColor(.green)
                    .cornerRadius(6)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.green.opacity(0.6), lineWidth: 1)
                    )
                }
            }
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.appSecondaryBackground)
                .overlay(
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(Color.yellow.opacity(0.3), lineWidth: 1)
                )
        )
        .shadow(color: Color.black.opacity(0.25), radius: 4, y: 2)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
    }

    private func detailRow(title: String, value: String) -> some View {
        HStack {
            Text("\(title):")
                .foregroundColor(.secondary)
            Text(value)
                .foregroundColor(.primary)
        }
    }

    private func triggerHaptic(success: Bool) {
        #if canImport(UIKit)
        let notification = UINotificationFeedbackGenerator()
        notification.prepare()
        notification.notificationOccurred(success ? .success : .error)
        #endif
    }
}
