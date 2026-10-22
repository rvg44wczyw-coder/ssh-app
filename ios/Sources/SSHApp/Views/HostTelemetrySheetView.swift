import SwiftUI
import UIKit
import SshCoreBridge

/// One screen per quick-menu tile: the host's numbers, its processes, Docker, the helper.
public enum HostScreen: String {
    case host, processes, docker, helper

    var title: String {
        switch self {
        case .host: return "Host monitor"
        case .processes: return "Processes"
        case .docker: return "Docker"
        case .helper: return "Host helper"
        }
    }

    /// Reads the telemetry snapshot (and refreshes it while open).
    var usesTelemetry: Bool { self == .host || self == .processes }
}

public struct HostTelemetrySheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var screen: HostScreen = .host
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

    public init(viewModel: TerminalSessionViewModel, screen: HostScreen = .host, onClose: (() -> Void)? = nil) {
        self.viewModel = viewModel
        self.screen = screen
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
                    switch screen {
                    case .docker:
                        DockerSection(viewModel: viewModel, closeSheet: closeSheet, autoLoad: true)
                    case .helper:
                        HostHelperCard(viewModel: viewModel)
                    case .host, .processes:
                        telemetryContent
                    }
                }
                .padding(16)
            }
            .background(Color.appBackground.ignoresSafeArea())
            .navigationTitle(screen.title)
            .task(id: refreshSeconds) {
                // Cancelled when the sheet goes away or the interval changes.
                guard screen.usesTelemetry, refreshSeconds > 0 else { return }
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
                if screen.usesTelemetry {
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
                    viewModel.killRemoteProcess(pid: proc.pid, force: true, started: proc.startedEpoch)
                    processToForceKill = nil
                }
            } message: { proc in
                Text("Immediately terminate '\(proc.name)' (PID \(proc.pid)) with SIGKILL? Unsaved state in this process may be lost.")
            }
            .onAppear {
                if screen.usesTelemetry { viewModel.refreshHostTelemetry() }
            }
        }
    }

    @ViewBuilder
    private var telemetryContent: some View {
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
            if screen == .host {
                hostSummaryCard(snap)
                resourceGaugesGrid(snap)
            } else {
                processMonitorSection(snap)
            }
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

            if let batt = snap.batteryPercent, !(viewModel.hostTarget.map { hostTargetHidesBattery(target: $0) } ?? false) {
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
                            Label(proc.elapsed, systemImage: "clock")
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                    }

                    Spacer()

                    if proc.pid > 1 {
                        Menu {
                            Button {
                                viewModel.killRemoteProcess(pid: proc.pid, force: false, started: proc.startedEpoch)
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


extension Notification.Name {
    /// Posted by the Docker section: open a tab with a shell in a container (userInfo: title, sessionName, command).
    static let openDockerTab = Notification.Name("ssh-app.open-docker-tab")
}

/// Docker containers of the host (feature 45). Nothing is asked of the host until the user opens
/// the section; Restart and Stop name the container and ask first.
struct DockerSection: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    var closeSheet: () -> Void
    /// The Docker screen asks for the list as it opens; nothing is asked of a host before that.
    var autoLoad = false

    @State private var snapshot: DockerContainersSnapshot?
    @State private var stats: [String: DockerContainerStats] = [:]
    @State private var loading = false
    @State private var busyId: String?
    @State private var message: String?
    @State private var pending: (action: DockerAction, container: DockerContainerInfo)?
    @State private var logsFor: DockerContainerInfo?
    @Environment(\.scenePhase) private var scenePhase

    private var handle: SshSessionHandle? { viewModel.state == .connected ? viewModel.sessionHandle : nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("Docker containers", systemImage: "shippingbox.fill").font(.headline)
                Spacer()
                Button(snapshot == nil ? "Show" : "Refresh") { Task { await load() } }
                    .disabled(loading || handle == nil)
            }
            if loading { ProgressView().controlSize(.small) }
            if handle == nil, snapshot == nil {
                Text("Not connected to the host. Docker is read from the host once the tab is connected.")
                    .font(.caption).foregroundColor(.secondary)
            }
            if let message { Text(message).font(.caption).foregroundColor(.orange) }
            if let snap = snapshot {
                if let why = snap.unavailable {
                    DockerUnavailableView(reason: why) {
                        Task { await load() }
                    }
                } else if snap.containers.isEmpty {
                    Text("No containers.").font(.caption).foregroundColor(.secondary)
                }
                ForEach(snap.containers, id: \.id) { c in card(c) }
                if snap.dockerAvailable {
                    Text("Membership in the docker group is root on the host. A shell inside a container is not covered by the hook guard.")
                        .font(.caption2).foregroundColor(.secondary)
                }
            }
        }
        .padding(14)
        .background(Color.appSecondaryBackground, in: RoundedRectangle(cornerRadius: 12))
        .task {
            if autoLoad, snapshot == nil, handle != nil { await load() }
        }
        .task(id: snapshot != nil) {
            // The list refreshes itself only while it is open, the sheet is up and the app is active.
            while snapshot != nil, !Task.isCancelled {
                let secs = AppConfigStore.current().docker.autoRefreshSecs
                try? await Task.sleep(nanoseconds: UInt64(secs == 0 ? 5 : secs) * 1_000_000_000)
                if secs > 0, !Task.isCancelled, !loading, busyId == nil, pending == nil, scenePhase == .active { await load() }
            }
        }
        .confirmationDialog(
            pending.map { "\(Self.verb($0.action)) \($0.container.name)?" } ?? "",
            isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
            titleVisibility: .visible
        ) {
            if let p = pending {
                Button(Self.verb(p.action), role: .destructive) { Task { await run(p.action, p.container) } }
            }
            Button("Cancel", role: .cancel) { pending = nil }
        } message: {
            Text("It may be a database or a service other people use.")
        }
        .sheet(item: Binding(get: { logsFor.map { DockerLogTarget(container: $0) } }, set: { if $0 == nil { logsFor = nil } })) { target in
            DockerLogsSheet(viewModel: viewModel, container: target.container)
        }
    }

    private static func verb(_ a: DockerAction) -> String {
        switch a {
        case .start: return "Start"
        case .stop: return "Stop"
        case .restart: return "Restart"
        }
    }

    private func color(_ s: ContainerState) -> Color {
        switch s {
        case .running: return .green
        case .exited, .dead: return .red
        case .paused, .restarting: return .orange
        case .unknown: return .secondary
        }
    }

    private func card(_ c: DockerContainerInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: c.state == .unknown ? "circle" : "circle.fill").font(.system(size: 9)).foregroundColor(color(c.state))
                Text(c.name).font(.system(size: 14, weight: .semibold, design: .monospaced)).lineLimit(1)
                if c.unhealthy { Text("unhealthy").font(.caption2).padding(.horizontal, 5).background(Color.orange.opacity(0.3), in: Capsule()) }
                Spacer()
                Text(c.statusText).font(.caption).foregroundColor(.secondary).lineLimit(1)
            }
            Text(c.image + (c.ports.isEmpty ? "" : "  \(c.ports)")).font(.caption).foregroundColor(.secondary).lineLimit(1)
            if let s = stats[c.id] {
                Text(statsLine(s)).font(.caption2).foregroundColor(.secondary)
            }
            HStack(spacing: 8) {
                if c.state == .running || c.state == .restarting || c.state == .paused {
                    action("Restart", "arrow.clockwise") { pending = (.restart, c) }
                    action("Stop", "stop.fill") { pending = (.stop, c) }
                } else {
                    action("Start", "play.fill") { Task { await run(.start, c) } }
                }
                action("Logs", "doc.text") { logsFor = c }
                action("Terminal", "terminal") { openTerminal(c) }
                if c.state == .running { action("Stats", "gauge") { Task { await loadStats(c) } } }
            }
            .disabled(busyId == c.id)
        }
        .padding(.vertical, 4)
    }

    private func statsLine(_ s: DockerContainerStats) -> String {
        let mb = { (b: UInt64) in "\(b / (1024 * 1024)) MB" }
        let mem = s.memoryUsageBytes.map { u in "RAM \(mb(u))" + (s.memoryLimitBytes.map { " / \(mb($0))" } ?? "") } ?? "RAM --"
        let cpu = s.cpuPercentage.map { String(format: "CPU %.1f%%", $0) } ?? "CPU --"
        return mem + "   " + cpu
    }

    private func action(_ title: String, _ icon: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) { Label(title, systemImage: icon).font(.caption).labelStyle(.titleAndIcon) }
            .buttonStyle(.bordered).controlSize(.mini)
    }

    private func load() async {
        guard let handle else { return }
        loading = true; message = nil
        defer { loading = false }
        do { snapshot = try await handle.fetchDockerContainers() }
        catch { message = error.localizedDescription }
    }

    private func loadStats(_ c: DockerContainerInfo) async {
        guard let handle else { return }
        busyId = c.id; defer { busyId = nil }
        if let s = try? await handle.fetchDockerStats(containerId: c.id) { stats[c.id] = s }
    }

    private func run(_ action: DockerAction, _ c: DockerContainerInfo) async {
        guard let handle else { return }
        busyId = c.id; message = nil
        defer { busyId = nil }
        do {
            try await handle.dockerContainerAction(action: action, containerId: c.id)
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            await load()
        } catch {
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            message = error.localizedDescription
        }
    }

    private func openTerminal(_ c: DockerContainerInfo) {
        do {
            let command = try buildDockerExecCommand(containerId: c.id)
            let name = try dockerSessionName(containerName: c.name, containerId: c.id)
            NotificationCenter.default.post(name: .openDockerTab, object: nil,
                                            userInfo: ["title": "[docker] \(c.name)", "sessionName": name, "command": command])
            closeSheet()
        } catch {
            message = error.localizedDescription
        }
    }
}

/// Docker cannot be used on this host: what is wrong, and the way out, worded for each cause.
struct DockerUnavailableView: View {
    let reason: DockerUnavailable
    var retry: () -> Void

    private var content: (icon: String, title: String, body: String, fix: String?) {
        switch reason {
        case .notInstalled:
            return ("shippingbox", "Docker is not installed",
                    "There is no docker command on this host, so there is nothing to list.",
                    "Install Docker Desktop, OrbStack or Colima on a Mac, or docker.io / docker-ce on Linux.")
        case .daemonDown:
            return ("poweroff", "Docker is not running",
                    "The docker command is there, but its engine does not answer.",
                    "Start Docker Desktop / OrbStack / Colima on the host (Linux: sudo systemctl start docker), then try again.")
        case .noPermission:
            return ("lock.shield", "No permission to use Docker",
                    "The login user of this host may not talk to the Docker engine.",
                    "Linux: sudo usermod -aG docker $USER, then sign in again. Being in the docker group is root-equivalent, so it is not done for you.")
        case .unexpected:
            return ("questionmark.circle", "Docker did not answer as expected",
                    "The host replied with something this app cannot read as a container list.",
                    "Run docker ps on the host to see what it says, then try again.")
        }
    }

    var body: some View {
        let c = content
        VStack(spacing: 8) {
            Image(systemName: c.icon).font(.system(size: 30)).foregroundColor(.secondary)
            Text(c.title).font(.headline)
            Text(c.body).font(.caption).foregroundColor(.secondary).multilineTextAlignment(.center)
            if let fix = c.fix {
                Text(fix).font(.caption).multilineTextAlignment(.center)
            }
            Button("Try again", action: retry).buttonStyle(.bordered).controlSize(.small)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
    }
}

private struct DockerLogTarget: Identifiable {
    let container: DockerContainerInfo
    var id: String { container.id }
}

/// The last lines of a container's log; Fetch Fresh asks again, Copy puts the text on the clipboard.
enum DockerLogFollow {
    static let intervalSeconds = 3.0
}

struct DockerLogsSheet: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    let container: DockerContainerInfo
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var loading = false
    @State private var error: String?
    @State private var follow = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(error ?? (text.isEmpty && !loading ? "No log lines." : text))
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundColor(error == nil ? .primary : .orange)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(12)
                        Color.clear.frame(height: 1).id("end")
                    }
                }
                .onChange(of: text) { _, _ in
                    if follow { proxy.scrollTo("end", anchor: .bottom) }
                }
            }
            .overlay { if loading && !follow { ProgressView() } }
            .navigationTitle("Logs: \(container.name)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { UIPasteboard.general.string = text } label: { Image(systemName: "doc.on.doc") }
                        .disabled(text.isEmpty).accessibilityLabel("Copy all")
                }
                ToolbarItem(placement: .bottomBar) { Button("Fetch fresh") { Task { await load() } }.disabled(loading) }
                ToolbarItem(placement: .bottomBar) { Toggle("Live", isOn: $follow) }
            }
            .task { await load() }
            // Live: fetches again every few seconds while the sheet is open and the app is in front;
            // the loop ends with the view, the toggle or the background, so nothing runs unseen.
            .task(id: "\(follow)|\(scenePhase == .active)") {
                guard follow, scenePhase == .active else { return }
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(DockerLogFollow.intervalSeconds))
                    if Task.isCancelled { break }
                    await load()
                }
            }
        }
    }

    private func load() async {
        guard let handle = viewModel.sessionHandle, viewModel.state == .connected else { error = "Not connected."; return }
        loading = true; error = nil
        defer { loading = false }
        do { text = try await handle.fetchDockerLogs(containerId: container.id, tailLines: AppConfigStore.current().docker.logTailLines) }
        catch { self.error = error.localizedDescription }
    }
}
