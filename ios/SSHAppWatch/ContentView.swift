import SwiftUI

struct ContentView: View {
    @EnvironmentObject var model: WatchModel

    var body: some View {
        NavigationStack {
            Group {
                if model.items.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: model.phoneReachable ? "checkmark.shield" : "iphone.slash")
                            .font(.largeTitle)
                            .foregroundStyle(model.phoneReachable ? .green : .orange)
                        Text(model.phoneReachable ? "Nothing to approve" : "iPhone not reachable")
                            .font(.headline)
                        if !model.phoneReachable {
                            Text("Open SSH Mobile on the iPhone to receive requests.")
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.center)
                        }
                    }
                } else {
                    List(model.items) { item in
                        NavigationLink(value: item.id) { row(item) }
                    }
                    .navigationDestination(for: String.self) { id in
                        if let item = model.items.first(where: { $0.id == id }) {
                            DetailView(item: item)
                        } else {
                            Text("Done").font(.headline)
                        }
                    }
                }
            }
            .navigationTitle("Approvals")
        }
    }

    private func row(_ item: WatchApproval) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(verbatim: item.tool).font(.headline)
                Spacer()
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    Text("\(model.remaining(item, now: ctx.date))s")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(model.remaining(item, now: ctx.date) < 20 ? .red : .secondary)
                }
            }
            Text(verbatim: item.tail).font(.caption2).lineLimit(2).foregroundStyle(.secondary)
            if !item.flags.isEmpty {
                Text(verbatim: item.flags.joined(separator: " · ")).font(.caption2).foregroundStyle(.orange).lineLimit(1)
            }
        }
    }
}

struct DetailView: View {
    @EnvironmentObject var model: WatchModel
    @Environment(\.dismiss) private var dismiss
    let item: WatchApproval
    @State private var confirmApprove = false

    private var sending: Bool {
        if case .sending(let id) = model.sendState { return id == item.id }
        return false
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                Text(verbatim: "\(item.agent) · \(item.server)").font(.caption2).foregroundStyle(.secondary)
                Text(verbatim: item.tool).font(.headline)
                Text(verbatim: item.tail).font(.system(size: 12, design: .monospaced))
                if !item.dir.isEmpty {
                    Text(verbatim: item.dir).font(.caption2).foregroundStyle(.secondary)
                }
                ForEach(item.flags, id: \.self) { flag in
                    Label(flag, systemImage: "exclamationmark.triangle.fill").font(.caption2).foregroundStyle(.orange)
                }
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    Text("\(model.remaining(item, now: ctx.date))s left").font(.caption.monospacedDigit())
                }
                if case .failed(let id) = model.sendState, id == item.id {
                    Text("iPhone did not answer. Open the app on the iPhone.").font(.caption2).foregroundStyle(.red)
                }
                if case .done(let id, let reply) = model.sendState, id == item.id, reply != .signed {
                    Text(message(reply)).font(.caption2).foregroundStyle(.red)
                }
                // Deny first: the safe answer is the one under the thumb.
                Button(role: .destructive) { send(false) } label: { Label("Deny", systemImage: "xmark") }
                    .disabled(sending)
                Button { item.flags.isEmpty ? send(true) : (confirmApprove = true) } label: {
                    Label("Approve", systemImage: "checkmark")
                }
                .tint(.green)
                .disabled(sending || !model.phoneReachable)
            }
        }
        .confirmationDialog("This request is flagged", isPresented: $confirmApprove, titleVisibility: .visible) {
            Button("Approve anyway") { send(true) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(verbatim: item.flags.joined(separator: ", "))
        }
    }

    private func send(_ approved: Bool) {
        model.send(item, approved: approved)
        dismiss()
    }

    private func message(_ reply: WatchReply) -> String {
        switch reply {
        case .signed: return ""
        case .expired: return "Expired - the agent asks again in its own prompt."
        case .notPending: return "Already answered."
        case .hostUnreachable: return "The host could not be reached."
        case .rejected: return "Rejected."
        }
    }
}
