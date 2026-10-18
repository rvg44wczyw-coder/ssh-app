import SwiftUI
import SshCoreBridge

public struct HostTelemetrySheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var selectedFilter: String = "All"
    /// 0 = off. Stops while the sheet is closed or the app is in the background (zero battery drain).
    @AppStorage("monitor_refresh_seconds") private var refreshSeconds: Int = 10
    @Environment(\.scenePhase) private var scenePhase
    @State private var processToForceKill: RemoteProcessInfo? = nil
    /// System and other-account processes start collapsed: the user-space group is what is usually wanted.
    @State private var collapsedGroups: Set<ProcessGroup> = [.system, .other]

    private let filters = ["All", "AI Agents", "Local LLM", "Build", "Runtime"]

    public init(viewModel: TerminalSessionViewModel, onClose: (() -> Void)? = nil) {
        self.viewModel = viewModel
        self.onClose = onClose
    }

    private func closeSheet() {
        onClose?()
        dismiss()
    }

    private var filteredProcesses: [RemoteProcessInfo] {
        guard let procs = viewModel.telemetrySnapshot?.processes else { return [] }
        switch selectedFilter {
        case "AI Agents":
            return procs.filter { $0.category == .aiAgent }
        case "Local LLM":
            return procs.filter { $0.category == .localLlm }
        case "Build":
            return procs.filter { $0.category == .buildTool }
        case "Runtime":
            return procs.filter { $0.category == .runtime }
        default:
            return procs
        }
    }

    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    if let error = viewModel.telemetryError {
                        HStack(spacing: 8) {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundColor(.yellow)
                            Text(error)
                                .font(.caption)
                                .foregroundColor(.white)
                            Spacer()
                        }
                        .padding(12)
                        .background(Color.red.opacity(0.25))
                        .cornerRadius(10)
                    }

                    if let snap = viewModel.telemetrySnapshot {
                        if let t = viewModel.telemetryUpdatedAt {
                            Text("Updated \(t.formatted(date: .omitted, time: .standard))" + (refreshSeconds > 0 ? " · every \(refreshSeconds) s" : " · auto-refresh off"))
                                .font(.caption2).foregroundColor(.secondary)
                                .frame(maxWidth: .infinity, alignment: .trailing)
                        }
                        hostSummaryCard(snap)
                        resourceGaugesGrid(snap)
                        processMonitorSection(snap)
                        HostHelperCard(viewModel: viewModel)
                    } else if viewModel.isTelemetryLoading {
                        VStack(spacing: 12) {
                            ProgressView()
                                .scaleEffect(1.1)
                            Text("Collecting remote host telemetry...")
                                .font(.subheadline)
                                .foregroundColor(.secondary)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 60)
                    } else {
                        VStack(spacing: 12) {
                            Image(systemName: "gauge.with.dots.needle.33percent")
                                .font(.system(size: 40))
                                .foregroundColor(.secondary)
                            Text("No Telemetry Snapshot")
                                .font(.headline)
                            Button("Refresh Now") {
                                viewModel.refreshHostTelemetry()
                            }
                            .buttonStyle(.borderedProminent)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 50)
                    }
                }
                .padding(16)
            }
            .background(Color.appBackground.ignoresSafeArea())
            .navigationTitle("Host Monitor")
            .task(id: refreshSeconds) {
                // Cancelled when the sheet goes away or the interval changes.
                guard refreshSeconds > 0 else { return }
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: UInt64(refreshSeconds) * 1_000_000_000)
                    if Task.isCancelled { break }
                    if scenePhase == .active, !viewModel.isTelemetryLoading { viewModel.refreshHostTelemetry() }
                }
            }
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") {
                        closeSheet()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Picker("Auto-refresh", selection: $refreshSeconds) {
                            Text("Off").tag(0)
                            Text("Every 5 s").tag(5)
                            Text("Every 10 s").tag(10)
                            Text("Every 30 s").tag(30)
                            Text("Every 60 s").tag(60)
                        }
                    } label: {
                        Image(systemName: refreshSeconds > 0 ? "timer" : "timer.slash")
                    }
                    .accessibilityLabel("Auto-refresh")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: { viewModel.refreshHostTelemetry() }) {
                        if viewModel.isTelemetryLoading {
                            ProgressView()
                                .scaleEffect(0.75)
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                    }
                    .accessibilityLabel("Refresh Telemetry")
                }
            }
            .alert(
                "Force Kill Process?",
                isPresented: Binding(
                    get: { processToForceKill != nil },
                    set: { if !$0 { processToForceKill = nil } }
                ),
                presenting: processToForceKill
            ) { proc in
                Button("Cancel", role: .cancel) {
                    processToForceKill = nil
                }
                Button("SIGKILL (-9)", role: .destructive) {
                    viewModel.killRemoteProcess(pid: proc.pid, force: true)
                    processToForceKill = nil
                }
            } message: { proc in
                Text("Immediately terminate '\(proc.name)' (PID \(proc.pid)) with SIGKILL? Unsaved state in this process may be lost.")
            }
            .onAppear {
                viewModel.refreshHostTelemetry()
            }
        }
    }

    // MARK: - Host Summary & Battery Card
    private func hostSummaryCard(_ snap: HostTelemetrySnapshot) -> some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.appAccent.opacity(0.18))
                    .frame(width: 44, height: 44)
                Image(systemName: "laptopcomputer")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundColor(.appAccent)
            }

            VStack(alignment: .leading, spacing: 3) {
                Text(snap.hostname)
                    .font(.system(size: 15, weight: .bold, design: .monospaced))
                    .foregroundColor(.primary)
                Text("\(snap.osName) • \(snap.cpuCores) Cores")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
            }

            Spacer()

            if let batt = snap.batteryPercent {
                VStack(alignment: .trailing, spacing: 3) {
                    HStack(spacing: 4) {
                        Image(systemName: Self.batterySymbol(percent: Int(batt), charging: snap.isCharging == true))
                            .foregroundColor(batt <= 20 && snap.isCharging != true ? .red : .green)
                        Text("\(batt)%")
                            .font(.system(size: 13, weight: .bold, design: .monospaced))
                    }
                    if let source = snap.powerSource {
                        Text(source)
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                }
            }
        }
        .padding(14)
        .background(Color.appSecondaryBackground)
        .cornerRadius(12)
    }

    /// SF Symbol that follows the charge level; the bolt variant while charging.
    static func batterySymbol(percent: Int, charging: Bool) -> String {
        if charging { return "battery.100.bolt" }
        switch percent {
        case ..<10: return "battery.0"
        case ..<38: return "battery.25"
        case ..<63: return "battery.50"
        case ..<88: return "battery.75"
        default: return "battery.100"
        }
    }

    // MARK: - CPU / RAM / Disk Gauges
    private func resourceGaugesGrid(_ snap: HostTelemetrySnapshot) -> some View {
        VStack(spacing: 10) {
            // CPU Load Card
            let loadRatio = min(1.0, snap.load1m / Double(max(1, snap.cpuCores)))
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Label("CPU Load Average", systemImage: "cpu")
                        .font(.system(size: 12, weight: .semibold))
                    Spacer()
                    Text(String(format: "%.2f / %.2f / %.2f", snap.load1m, snap.load5m, snap.load15m))
                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                        .foregroundColor(loadRatio > 0.85 ? .red : (loadRatio > 0.6 ? .orange : .green))
                }
                ProgressView(value: loadRatio)
                    .tint(loadRatio > 0.85 ? .red : (loadRatio > 0.6 ? .orange : .green))
            }
            .padding(12)
            .background(Color.appSecondaryBackground)
            .cornerRadius(10)

            // Memory Usage Card
            let memRatio = min(1.0, max(0.0, snap.memoryUsagePercent / 100.0))
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Label("Memory (RAM)", systemImage: "memorychip")
                        .font(.system(size: 12, weight: .semibold))
                    Spacer()
                    Text(String(format: "%llu / %llu MB (%.0f%%)", snap.memoryUsedMb, snap.memoryTotalMb, snap.memoryUsagePercent))
                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                        .foregroundColor(memRatio > 0.85 ? .red : (memRatio > 0.7 ? .orange : .blue))
                }
                ProgressView(value: memRatio)
                    .tint(memRatio > 0.85 ? .red : (memRatio > 0.7 ? .orange : .blue))
            }
            .padding(12)
            .background(Color.appSecondaryBackground)
            .cornerRadius(10)

            // Disk Usage Card
            let diskRatio = min(1.0, max(0.0, snap.diskUsagePercent / 100.0))
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Label("Root Disk (/)", systemImage: "internaldrive")
                        .font(.system(size: 12, weight: .semibold))
                    Spacer()
                    Text(String(format: "%.1f / %.1f GB (%.0f%%)", snap.diskUsedGb, snap.diskTotalGb, snap.diskUsagePercent))
                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                        .foregroundColor(diskRatio > 0.9 ? .red : .secondary)
                }
                ProgressView(value: diskRatio)
                    .tint(diskRatio > 0.9 ? .red : .purple)
            }
            .padding(12)
            .background(Color.appSecondaryBackground)
            .cornerRadius(10)
        }
    }

    // MARK: - Process Monitor
    private let groupOrder: [ProcessGroup] = [.user, .system, .other]

    private func groupTitle(_ g: ProcessGroup) -> String {
        switch g {
        case .user: return "User-space"
        case .system: return "System"
        case .other: return "Other"
        }
    }

    private func groupHint(_ g: ProcessGroup) -> String {
        switch g {
        case .user: return "started by the account the app logs in with"
        case .system: return "root, service accounts and OS daemons"
        case .other: return "other accounts"
        }
    }

    private func groupHeader(_ g: ProcessGroup, count: Int) -> some View {
        let collapsed = collapsedGroups.contains(g)
        return Button {
            if collapsed { collapsedGroups.remove(g) } else { collapsedGroups.insert(g) }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: collapsed ? "chevron.right" : "chevron.down").font(.system(size: 10, weight: .bold))
                Text("\(groupTitle(g)) (\(count))").font(.system(size: 12, weight: .bold))
                Text(groupHint(g)).font(.system(size: 10)).foregroundColor(.secondary).lineLimit(1)
                Spacer()
            }
            .foregroundColor(.primary)
            .padding(.top, 4)
        }
        .buttonStyle(.plain)
    }

    private func processRow(_ proc: RemoteProcessInfo) -> some View {
                HStack(spacing: 10) {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            categoryBadge(proc.category)
                            Text(proc.name)
                                .font(.system(size: 13, weight: .bold, design: .monospaced))
                                .lineLimit(1)
                            Text("PID \(proc.pid) · \(proc.owner)")
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary)
                        }

                        Text(proc.commandLine)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                            .lineLimit(1)

                        HStack(spacing: 12) {
                            Text(String(format: "CPU %.1f%%", proc.cpuPercent))
                                .font(.system(size: 10, weight: .semibold, design: .monospaced))
                                .foregroundColor(proc.cpuPercent > 50 ? .orange : .green)
                            Text(String(format: "MEM %.1f%%", proc.memPercent))
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary)
                            Text("⏱ \(proc.elapsed)")
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                    }

                    Spacer()

                    if proc.pid > 1 {
                        Menu {
                            Button {
                                viewModel.killRemoteProcess(pid: proc.pid, force: false)
                            } label: {
                                Label("Terminate (SIGTERM)", systemImage: "stop.circle")
                            }

                            Button(role: .destructive) {
                                processToForceKill = proc
                            } label: {
                                Label("Force Kill (SIGKILL -9)", systemImage: "xmark.octagon.fill")
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                                .font(.system(size: 18))
                                .foregroundColor(.secondary)
                                .padding(4)
                        }
                    }
                }
                .padding(10)
                .background(Color.appSecondaryBackground)
                .cornerRadius(10)
    }

    private func processMonitorSection(_ snap: HostTelemetrySnapshot) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("ACTIVE AGENTS & PROCESSES")
                    .font(.system(size: 11, weight: .bold, design: .monospaced))
                    .foregroundColor(.secondary)
                Spacer()
                Text("\(filteredProcesses.count) shown")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
            }

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(filters, id: \.self) { filter in
                        Button(action: { selectedFilter = filter }) {
                            Text(filter)
                                .font(.system(size: 11, weight: selectedFilter == filter ? .bold : .medium))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(selectedFilter == filter ? Color.appAccent : Color.appSecondaryBackground)
                                .foregroundColor(selectedFilter == filter ? .white : .primary)
                                .cornerRadius(14)
                        }
                    }
                }
            }

            if selectedFilter == "All" {
                ForEach(groupOrder, id: \.self) { group in
                    let items = filteredProcesses.filter { $0.group == group }
                    if !items.isEmpty {
                        groupHeader(group, count: items.count)
                        if !collapsedGroups.contains(group) {
                            ForEach(items, id: \.pid) { proc in processRow(proc) }
                        }
                    }
                }
            } else {
                ForEach(filteredProcesses, id: \.pid) { proc in processRow(proc) }
            }
        }
    }

    @ViewBuilder
    private func categoryBadge(_ category: ProcessCategory) -> some View {
        let (title, color): (String, Color) = {
            switch category {
            case .aiAgent:
                return ("AI AGENT", .purple)
            case .localLlm:
                return ("LLM", .cyan)
            case .buildTool:
                return ("BUILD", .orange)
            case .runtime:
                return ("RUNTIME", .green)
            case .system:
                return ("SYS", .gray)
            }
        }()

        Text(title)
            .font(.system(size: 9, weight: .bold, design: .monospaced))
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(color.opacity(0.2))
            .foregroundColor(color)
            .cornerRadius(4)
    }
}
