import SwiftUI
import SshCoreBridge

/// Card for the oldest pending approval: who is asking, where, the full command, a countdown on
/// the host's clock, and Approve / Deny. Host-controlled text is shown verbatim in a monospaced
/// scroll box (never as markdown or links); the core already replaced control and bidi
/// characters with visible markers.
public struct ApprovalBannerView: View {
    public let entry: ApprovalEntry
    public let queuedBehind: Int
    public let isBusy: Bool
    public let errorMessage: String?
    public let onDecision: (Bool) -> Void
    /// Set when a temporary trust can be offered for this request (feature 50).
    public let trustOffer: AutoTrustOffer?
    public let onTrust: ([String], UInt32) -> Void

    @State private var showFullCwd = false
    @AppStorage("trust_never_offer") private var neverOfferTrust = false
    @AppStorage("trust_default_secs") private var defaultTrustSecs = 3600

    public init(entry: ApprovalEntry, queuedBehind: Int, isBusy: Bool, errorMessage: String?,
                trustOffer: AutoTrustOffer? = nil,
                onTrust: @escaping ([String], UInt32) -> Void = { _, _ in },
                onDecision: @escaping (Bool) -> Void) {
        self.trustOffer = trustOffer
        self.onTrust = onTrust
        self.entry = entry
        self.queuedBehind = queuedBehind
        self.isBusy = isBusy
        self.errorMessage = errorMessage
        self.onDecision = onDecision
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "shield.lefthalf.filled.trianglebadge.exclamationmark")
                    .foregroundColor(.yellow)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(verbatim: entry.item.agent.capitalized)
                            .font(.system(size: 12, weight: .bold))
                            .foregroundColor(.appAccent)
                        Text(verbatim: "wants to use \(entry.item.tool)")
                            .font(.system(size: 12, weight: .medium))
                    }
                    Text(verbatim: "\(entry.serverName) · \(showFullCwd ? entry.item.cwd : entry.shortCwd)")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.secondary)
                        .lineLimit(showFullCwd ? nil : 1)
                        .truncationMode(.middle)
                        .onTapGesture { showFullCwd.toggle() }
                        .accessibilityHint("Tap to show the full working directory")
                }
                Spacer()
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let left = entry.remainingSeconds(at: context.date)
                    Text(verbatim: "\(left)s")
                        .font(.system(size: 12, weight: .semibold, design: .monospaced))
                        .foregroundColor(left <= 15 ? .red : .secondary)
                        .accessibilityLabel("\(left) seconds left")
                }
            }

            let analysis = analyzeCommand(command: entry.item.command)
            if !analysis.flags.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(analysis.flags, id: \.self) { flag in
                        Label(flag, systemImage: "exclamationmark.triangle.fill")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(.orange)
                    }
                }
            }
            if analysis.length > 0 {
                Text(verbatim: "\(analysis.length) characters" + (analysis.lines > 1 ? ", \(analysis.lines) lines" : ""))
                    .font(.system(size: 9))
                    .foregroundColor(.secondary)
            }
            ScrollView {
                Text(verbatim: entry.item.command.isEmpty ? "(no command text)" : entry.item.command)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundColor(.green)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
            .frame(maxHeight: 140)
            .padding(6)
            .background(Color.black.opacity(0.4))
            .cornerRadius(6)
            if !analysis.tail.isEmpty {
                // A long command can hide its end below the scroll box, so the end is shown on its own.
                VStack(alignment: .leading, spacing: 2) {
                    Text("Ends with:").font(.system(size: 9)).foregroundColor(.secondary)
                    Text(verbatim: analysis.tail)
                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                        .foregroundColor(.yellow)
                        .lineLimit(3)
                        .truncationMode(.head)
                }
            }

            if let errorMessage {
                Text(verbatim: errorMessage)
                    .font(.caption2)
                    .foregroundColor(.red)
            }

            HStack(spacing: 10) {
                decisionButton(title: "Deny", icon: "xmark", color: .red, approved: false)
                decisionButton(title: "Approve", icon: "checkmark.shield.fill", color: .green, approved: true)
            }

            if let offer = trustOffer, !neverOfferTrust {
                trustMenu(offer)
            }

            if queuedBehind > 0 {
                Text(verbatim: "+\(queuedBehind) more waiting")
                    .font(.caption2)
                    .foregroundColor(.secondary)
            }
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.appSecondaryBackground)
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.yellow.opacity(0.4), lineWidth: 1))
        )
        .shadow(color: Color.black.opacity(0.25), radius: 4, y: 2)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
    }

    private static func durationName(_ secs: Int) -> String {
        secs % 3600 == 0 ? "\(secs / 3600) h" : "\(secs / 60) min"
    }

    /// "Approve, and allow Rust in ssh-app for 1 h": the host keeps a signed trust for this agent
    /// session and this project only; it never extends past the project folder.
    private func trustMenu(_ offer: AutoTrustOffer) -> some View {
        let titles = autoCategories().filter { offer.categories.contains($0.id) }.map(\.title).joined(separator: ", ")
        let durations = [900, 3600, 4 * 3600, Int(offer.maxSecs)]
            .filter { $0 <= Int(offer.maxSecs) }
        var unique: [Int] = []
        for d in durations where !unique.contains(d) { unique.append(d) }
        return Menu {
            ForEach(unique, id: \.self) { secs in
                Button(secs == Int(offer.maxSecs) && secs > 4 * 3600 ? "Until the session ends (max \(Self.durationName(secs)))" : "For \(Self.durationName(secs))") {
                    onTrust(offer.categories, UInt32(secs))
                }
            }
        } label: {
            Label("Approve, and allow \(titles) in \(offer.projectLabel)…", systemImage: "clock.badge.checkmark")
                .font(.system(size: 11, weight: .semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(Color.green.opacity(0.12))
                .foregroundColor(.green)
                .cornerRadius(6)
        }
        .disabled(isBusy)
    }

    private func decisionButton(title: String, icon: String, color: Color, approved: Bool) -> some View {
        Button {
            onDecision(approved)
        } label: {
            HStack(spacing: 4) {
                if isBusy {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: icon).font(.system(size: 11, weight: .bold))
                }
                Text(title).font(.system(size: 13, weight: .bold))
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(color.opacity(0.22))
            .foregroundColor(color)
            .cornerRadius(6)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(color.opacity(0.5), lineWidth: 1))
        }
        .disabled(isBusy)
    }
}
