import SwiftUI

public enum ServerReachability: Equatable, Sendable {
    case checking, online, offline
}

/// Centered popup for choosing a server: dimmed backdrop, one row per server with its address
/// and reachability, a checkmark on the current one. Replaces the old confirmation dialog and
/// the dropdown inside the tmux sheet.
public struct ServerPickerPopup: View {
    public let title: String
    public let subtitle: String
    public let servers: [ServerProfile]
    public let statuses: [UUID: ServerReachability]
    public let currentId: UUID?
    public let onSelect: (ServerProfile) -> Void
    public let onCancel: () -> Void

    public init(
        title: String,
        subtitle: String,
        servers: [ServerProfile],
        statuses: [UUID: ServerReachability] = [:],
        currentId: UUID? = nil,
        onSelect: @escaping (ServerProfile) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.title = title
        self.subtitle = subtitle
        self.servers = servers
        self.statuses = statuses
        self.currentId = currentId
        self.onSelect = onSelect
        self.onCancel = onCancel
    }

    public var body: some View {
        ZStack {
            Color.black.opacity(0.45)
                .ignoresSafeArea()
                .onTapGesture(perform: onCancel)
                .accessibilityHidden(true)

            VStack(spacing: 14) {
                VStack(spacing: 4) {
                    Text(title).font(.headline)
                    Text(subtitle)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.center)
                }
                // Grows to its content; scrolls only when there are too many servers to fit.
                ViewThatFits(in: .vertical) {
                    rows
                    ScrollView { rows }.frame(maxHeight: 280)
                }
                Text("Cancel")
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.accentColor)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .contentShape(Rectangle())
                    .onTapGesture(perform: onCancel)
                    .accessibilityAddTraits(.isButton)
            }
            .padding(18)
            .frame(maxWidth: 340)
            .background(RoundedRectangle(cornerRadius: 18).fill(Color.appSecondaryBackground))
            .shadow(radius: 20)
            .padding(24)
            .accessibilityElement(children: .contain)
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape, onCancel)
        }
        .transition(.opacity.combined(with: .scale(scale: 0.96)))
    }

    private var rows: some View {
        VStack(spacing: 8) {
            ForEach(servers) { server in
                row(server)
            }
        }
    }

    @ViewBuilder
    private func row(_ server: ServerProfile) -> some View {
        let state = statuses[server.id] ?? .checking
        HStack(spacing: 10) {
            Circle()
                .fill(color(for: state))
                .frame(width: 10, height: 10)
            VStack(alignment: .leading, spacing: 2) {
                Text(server.name).font(.subheadline.weight(.semibold)).foregroundColor(.primary)
                Text(verbatim: "\(server.username)@\(server.host):\(server.port)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer()
            if server.id == currentId {
                Image(systemName: "checkmark").foregroundColor(.accentColor)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.primary.opacity(0.06)))
        .contentShape(Rectangle())
        .onTapGesture { onSelect(server) }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(server.name), \(label(for: state))\(server.id == currentId ? ", current" : "")")
    }

    private func color(for state: ServerReachability) -> Color {
        switch state {
        case .online: return .green
        case .offline: return .red
        case .checking: return .gray
        }
    }

    private func label(for state: ServerReachability) -> String {
        switch state {
        case .online: return "online"
        case .offline: return "offline"
        case .checking: return "checking"
        }
    }
}

/// Pings every server in parallel and reports each result as soon as it arrives.
public func probeServers(_ servers: [ServerProfile], timeout: TimeInterval = 3.0,
                         update: @escaping @MainActor (UUID, ServerReachability) -> Void) async {
    await withTaskGroup(of: (UUID, Bool).self) { group in
        for s in servers {
            group.addTask { (s.id, await ServerPingService.ping(host: s.host, port: s.port, timeout: timeout)) }
        }
        for await (id, ok) in group {
            await update(id, ok ? .online : .offline)
        }
    }
}
