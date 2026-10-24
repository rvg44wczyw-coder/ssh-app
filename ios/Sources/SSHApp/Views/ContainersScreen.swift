import SwiftUI
import UIKit
import SshCoreBridge

extension Notification.Name {
    /// Posted by the Containers screen: open a tab with a shell in a container, pod or instance
    /// (userInfo: title, sessionName, command).
    static let openDockerTab = Notification.Name("ssh-app.open-docker-tab")
}

/// Which runtime the user last looked at on a server, and which ones they confirmed on a real host.
enum ContainerRuntimePrefs {
    static func chosen(server: UUID?) -> ContainerRuntime? {
        UserDefaults.standard.string(forKey: "container_runtime_\(server?.uuidString ?? "-")").flatMap(parse)
    }

    static func choose(_ runtime: ContainerRuntime, server: UUID?) {
        UserDefaults.standard.set(name(runtime), forKey: "container_runtime_\(server?.uuidString ?? "-")")
    }

    static func isVerified(_ runtime: ContainerRuntime, server: UUID?) -> Bool {
        runtime == .docker || UserDefaults.standard.bool(forKey: "container_verified_\(server?.uuidString ?? "-")_\(name(runtime))")
    }

    static func markVerified(_ runtime: ContainerRuntime, server: UUID?) {
        UserDefaults.standard.set(true, forKey: "container_verified_\(server?.uuidString ?? "-")_\(name(runtime))")
    }

    static func name(_ r: ContainerRuntime) -> String {
        switch r {
        case .docker: return "docker"
        case .podman: return "podman"
        case .nerdctl: return "nerdctl"
        case .kubernetes: return "kubernetes"
        case .incus: return "incus"
        case .lxd: return "lxd"
        }
    }

    static func parse(_ s: String) -> ContainerRuntime? {
        [ContainerRuntime.docker, .podman, .nerdctl, .kubernetes, .incus, .lxd].first { name($0) == s }
    }

    static func title(_ r: ContainerRuntime) -> String {
        switch r {
        case .docker: return "Docker"
        case .podman: return "Podman"
        case .nerdctl: return "nerdctl"
        case .kubernetes: return "Kubernetes"
        case .incus: return "Incus"
        case .lxd: return "LXD"
        }
    }

    /// What the things of this runtime are called on the screen.
    static func noun(_ r: ContainerRuntime) -> String {
        switch r {
        case .kubernetes: return "pod"
        case .incus, .lxd: return "instance"
        default: return "container"
        }
    }
}

/// A group on the Containers screen: a Compose project (engines) or a namespace (Kubernetes).
struct ContainerGroup: Identifiable, Equatable {
    var id: String { project ?? title }
    let title: String
    /// Set for a Compose project, which has actions of its own.
    let project: String?
    let items: [DockerContainerInfo]

    /// Engines group by Compose project (loose containers last), Kubernetes by namespace; the rest
    /// is one list. A single unnamed group is shown without a header.
    static func make(_ containers: [DockerContainerInfo], runtime: ContainerRuntime) -> [ContainerGroup] {
        let caps = runtimeCaps(runtime: runtime)
        guard caps.groups else { return [ContainerGroup(title: "", project: nil, items: containers)] }
        if runtime == .kubernetes {
            let by = Dictionary(grouping: containers, by: \.scope)
            return by.keys.sorted().map { ContainerGroup(title: $0, project: nil, items: by[$0] ?? []) }
        }
        let by = Dictionary(grouping: containers.filter { $0.project != nil }, by: { $0.project ?? "" })
        var out = by.keys.sorted().map { ContainerGroup(title: $0, project: $0, items: by[$0] ?? []) }
        let loose = containers.filter { $0.project == nil }
        if !loose.isEmpty { out.append(ContainerGroup(title: out.isEmpty ? "" : "Not in a stack", project: nil, items: loose)) }
        return out
    }
}

/// What to show when the answer of a stack action came back.
enum StackMessage {
    static func text(_ o: StackOutcome, names: [String: String]) -> String {
        func n(_ id: String) -> String { names[id] ?? id }
        if let failed = o.failedId {
            var s = "Done: \(o.done.count). Failed: \(n(failed))" + ((o.failedReason).map { $0.isEmpty ? "" : " (\($0))" } ?? "")
            if !o.notStarted.isEmpty { s += ". Not started: \(o.notStarted.map(n).joined(separator: ", "))" }
            return s + "."
        }
        if o.unknown { return "The host did not finish answering: the stack may be partly done. The list below is the truth." }
        return "Done: \(o.done.count)."
    }
}

/// Containers of the host (features 45 and 51): Docker, Podman, nerdctl, Compose stacks, Kubernetes
/// pods and Incus/LXD instances. Nothing is asked of the host until the screen opens; every action
/// names what it touches and asks first.
struct ContainersSection: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    var closeSheet: () -> Void
    /// The screen asks for the list as it opens; nothing is asked of a host before that.
    var autoLoad = false

    @State private var runtimes: [ContainerRuntime]?
    @State private var runtime: ContainerRuntime = .docker
    @State private var snapshot: DockerContainersSnapshot?
    @State private var stats: [String: DockerContainerStats] = [:]
    @State private var loading = false
    @State private var busyId: String?
    @State private var message: String?
    @State private var pending: (action: DockerAction, container: DockerContainerInfo)?
    @State private var pendingStack: (action: DockerAction, group: ContainerGroup)?
    @State private var logsFor: DockerContainerInfo?
    /// Bumped when the user confirms a runtime, so the "not seen yet" note redraws.
    @State private var verifiedTick = 0
    @Environment(\.scenePhase) private var scenePhase

    private var handle: SshSessionHandle? { viewModel.state == .connected ? viewModel.sessionHandle : nil }
    private var serverId: UUID? { viewModel.session.serverId }
    private var caps: RuntimeCaps { runtimeCaps(runtime: runtime) }
    private var noun: String { ContainerRuntimePrefs.noun(runtime) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(ContainerRuntimePrefs.title(runtime), systemImage: "shippingbox.fill").font(.headline)
                Spacer()
                Button(snapshot == nil ? "Show" : "Refresh") { Task { await load() } }
                    .disabled(loading || handle == nil)
            }
            if let runtimes, runtimes.count > 1 {
                Picker("Runtime", selection: Binding(get: { runtime }, set: { switchTo($0) })) {
                    ForEach(runtimes, id: \.self) { Text(ContainerRuntimePrefs.title($0)).tag($0) }
                }
                .pickerStyle(.segmented)
            }
            if loading { ProgressView().controlSize(.small) }
            if handle == nil, snapshot == nil {
                Text("Not connected to the host. Containers are read from the host once the tab is connected.")
                    .font(.caption).foregroundColor(.secondary)
            }
            if let runtimes, runtimes.isEmpty {
                ContainerUnavailableView(runtime: .docker, reason: .notInstalled, noRuntimeAtAll: true) { Task { await load() } }
            }
            if let message { Text(message).font(.caption).foregroundColor(.orange) }
            if verifiedTick >= 0, !ContainerRuntimePrefs.isVerified(runtime, server: serverId), snapshot?.dockerAvailable == true {
                unverifiedNote
            }
            if let snap = snapshot { content(snap) }
        }
        .padding(14)
        .glassCard(cornerRadius: 18)
        .task {
            if autoLoad, snapshot == nil, handle != nil { await load() }
        }
        .task(id: snapshot != nil) {
            // The list refreshes itself only while it is open, the sheet is up and the app is active.
            while snapshot != nil, !Task.isCancelled {
                let secs = AppConfigStore.current().docker.autoRefreshSecs
                try? await Task.sleep(nanoseconds: UInt64(secs == 0 ? 5 : secs) * 1_000_000_000)
                if secs > 0, !Task.isCancelled, !loading, busyId == nil, pending == nil, pendingStack == nil, scenePhase == .active { await refresh() }
            }
        }
        .confirmationDialog(
            pending.map { "\(Self.verb($0.action, runtime)) \($0.container.name)?" } ?? "",
            isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
            titleVisibility: .visible
        ) {
            if let p = pending {
                Button(Self.verb(p.action, runtime), role: .destructive) { Task { await run(p.action, p.container) } }
            }
            Button("Cancel", role: .cancel) { pending = nil }
        } message: {
            if runtime == .kubernetes, let p = pending {
                Text("The pod \(p.container.name) in \(p.container.scope) is deleted and its controller starts a new one.")
            } else {
                Text("It may be a database or a service other people use.")
            }
        }
        .confirmationDialog(
            pendingStack.map { "\(Self.verb($0.action, runtime)) \($0.group.items.count) \(noun)s of \($0.group.title)?" } ?? "",
            isPresented: Binding(get: { pendingStack != nil }, set: { if !$0 { pendingStack = nil } }),
            titleVisibility: .visible
        ) {
            if let p = pendingStack {
                Button(Self.verb(p.action, runtime) + " all", role: .destructive) { Task { await runStack(p.action, p.group) } }
            }
            Button("Cancel", role: .cancel) { pendingStack = nil }
        } message: {
            Text("They run one after another in the order of the list, which does not follow dependencies. It stops at the first failure.")
        }
        .sheet(item: Binding(get: { logsFor.map { DockerLogTarget(container: $0) } }, set: { if $0 == nil { logsFor = nil } })) { target in
            DockerLogsSheet(viewModel: viewModel, container: target.container, runtime: runtime)
        }
    }

    // MARK: Content

    @ViewBuilder
    private func content(_ snap: DockerContainersSnapshot) -> some View {
        if let why = snap.unavailable {
            ContainerUnavailableView(runtime: runtime, reason: why, detail: snap.errorMessage) { Task { await load() } }
        } else {
            if let note = snap.errorMessage { Text(note).font(.caption).foregroundColor(.secondary) }
            if snap.containers.isEmpty { Text("No \(noun)s.").font(.caption).foregroundColor(.secondary) }
            ForEach(ContainerGroup.make(snap.containers, runtime: runtime)) { group in
                if !group.title.isEmpty { groupHeader(group) }
                ForEach(group.items, id: \.id) { c in card(c) }
            }
            if Int(snap.total) > snap.containers.count {
                Text("Showing \(snap.containers.count) of \(snap.total).").font(.caption2).foregroundColor(.secondary)
            }
            if snap.unreadable > 0 {
                Text("\(snap.unreadable) entries could not be read.").font(.caption2).foregroundColor(.orange)
            }
            if runtime != .kubernetes {
                Text(footer).font(.caption2).foregroundColor(.secondary)
            }
        }
    }

    private var footer: String {
        switch runtime {
        case .incus, .lxd:
            return "Membership in the incus-admin group is root on the host. A shell inside an instance is not covered by the hook guard."
        default:
            return "Membership in the docker group is root on the host. A shell inside a container is not covered by the hook guard."
        }
    }

    private var unverifiedNote: some View {
        HStack {
            Text("\(ContainerRuntimePrefs.title(runtime)) support has not been seen on a real host yet.")
                .font(.caption2).foregroundColor(.secondary)
            Spacer()
            Button("It works") {
                ContainerRuntimePrefs.markVerified(runtime, server: serverId)
                verifiedTick += 1
            }
            .font(.caption2).buttonStyle(.glass).controlSize(.mini)
        }
    }

    private func groupHeader(_ g: ContainerGroup) -> some View {
        HStack {
            Image(systemName: g.project == nil ? "folder" : "square.stack.3d.up").foregroundColor(.secondary)
            Text(g.title).font(.subheadline.weight(.semibold))
            Text("\(g.items.count)").font(.caption).foregroundColor(.secondary)
            Spacer()
            if g.project != nil, caps.restart {
                Menu {
                    Button("Restart all", systemImage: "arrow.clockwise") { pendingStack = (.restart, g) }
                    if caps.startStop {
                        Button("Stop all", systemImage: "stop.fill") { pendingStack = (.stop, g) }
                        Button("Start all", systemImage: "play.fill") { pendingStack = (.start, g) }
                    }
                } label: { Image(systemName: "ellipsis.circle") }
                .disabled(busyId != nil || g.items.count > 20)
                .accessibilityLabel("Actions for the stack \(g.title)")
            }
        }
        .padding(.top, 6)
    }

    private static func verb(_ a: DockerAction, _ r: ContainerRuntime) -> String {
        switch a {
        case .start: return "Start"
        case .stop: return "Stop"
        case .restart: return r == .kubernetes ? "Delete (restart)" : "Restart"
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
                if let service = c.service, service != c.name {
                    Text(service).font(.caption2).padding(.horizontal, 5).glassChip()
                }
                if c.unhealthy { Text("unhealthy").font(.caption2).padding(.horizontal, 5).glassChip() }
                Spacer()
                Text(c.statusText).font(.caption).foregroundColor(.secondary).lineLimit(1)
            }
            Text(c.image + (c.ports.isEmpty ? "" : "  \(c.ports)")).font(.caption).foregroundColor(.secondary).lineLimit(1)
            if let s = stats[c.id] {
                Text(statsLine(s)).font(.caption2).foregroundColor(.secondary)
            }
            if runtime == .kubernetes, !c.restartable {
                Text("No controller: deleting this pod would lose it.").font(.caption2).foregroundColor(.secondary)
            }
            HStack(spacing: 8) {
                let up = c.state == .running || c.state == .restarting || c.state == .paused
                if caps.restart, c.restartable, up || runtime == .kubernetes {
                    action(Self.verb(.restart, runtime), "arrow.clockwise") { pending = (.restart, c) }
                }
                if caps.startStop {
                    if up { action("Stop", "stop.fill") { pending = (.stop, c) } }
                    else { action("Start", "play.fill") { Task { await run(.start, c) } } }
                }
                if caps.logs { action("Logs", "doc.text") { logsFor = c } }
                if caps.shell { action("Terminal", "terminal") { openTerminal(c) } }
                if caps.stats, c.state == .running { action("Stats", "gauge") { Task { await loadStats(c) } } }
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
            .buttonStyle(.glass).controlSize(.mini)
    }

    // MARK: Host calls

    private func switchTo(_ r: ContainerRuntime) {
        guard r != runtime else { return }
        runtime = r
        ContainerRuntimePrefs.choose(r, server: serverId)
        snapshot = nil; stats = [:]; message = nil
        Task { await load() }
    }

    /// First load: asks which runtimes the host has, then lists with the remembered or first one.
    private func load() async {
        guard let handle else { return }
        loading = true; message = nil
        defer { loading = false }
        do {
            if runtimes == nil {
                let found = try await handle.probeContainerRuntimes()
                runtimes = found
                if let remembered = ContainerRuntimePrefs.chosen(server: serverId), found.contains(remembered) {
                    runtime = remembered
                } else if let first = found.first {
                    runtime = first
                }
                if found.isEmpty { snapshot = nil; return }
            }
            let asked = runtime
            let fresh = try await handle.fetchRuntimeContainers(runtime: asked)
            // The user may have switched runtime while the host was answering.
            if asked == runtime { snapshot = fresh }
        } catch { message = error.localizedDescription }
    }

    private func refresh() async {
        guard let handle, runtimes != nil else { return }
        loading = true
        defer { loading = false }
        let asked = runtime
        do {
            let fresh = try await handle.fetchRuntimeContainers(runtime: asked)
            if asked == runtime { snapshot = fresh }
        } catch { message = error.localizedDescription }
    }

    private func loadStats(_ c: DockerContainerInfo) async {
        guard let handle else { return }
        busyId = c.id; defer { busyId = nil }
        if let s = try? await handle.fetchRuntimeStats(runtime: runtime, containerId: c.id) { stats[c.id] = s }
    }

    private func run(_ action: DockerAction, _ c: DockerContainerInfo) async {
        guard let handle else { return }
        busyId = c.id; message = nil
        defer { busyId = nil }
        do {
            try await handle.runtimeContainerAction(runtime: runtime, action: action, containerId: c.id)
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            await refresh()
        } catch {
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            message = error.localizedDescription
        }
    }

    private func runStack(_ action: DockerAction, _ group: ContainerGroup) async {
        guard let handle else { return }
        busyId = "stack:\(group.id)"; message = nil
        defer { busyId = nil }
        let names = Dictionary(uniqueKeysWithValues: group.items.map { ($0.id, $0.name) })
        do {
            let outcome = try await handle.runtimeStackAction(runtime: runtime, action: action, containerIds: group.items.map(\.id))
            message = StackMessage.text(outcome, names: names)
            let ok = outcome.failedId == nil && !outcome.unknown
            UINotificationFeedbackGenerator().notificationOccurred(ok ? .success : .warning)
        } catch {
            UINotificationFeedbackGenerator().notificationOccurred(.error)
            message = error.localizedDescription
        }
        await refresh()
    }

    private func openTerminal(_ c: DockerContainerInfo) {
        do {
            let command = try buildRuntimeExecCommand(runtime: runtime, containerId: c.id)
            let name = try runtimeSessionName(runtime: runtime, containerName: c.name, containerId: c.id)
            NotificationCenter.default.post(name: .openDockerTab, object: nil,
                                            userInfo: ["title": "[\(ContainerRuntimePrefs.name(runtime))] \(c.name)", "sessionName": name, "command": command])
            closeSheet()
        } catch {
            message = error.localizedDescription
        }
    }
}

struct DockerLogTarget: Identifiable {
    let container: DockerContainerInfo
    var id: String { container.id }
}

/// The runtime cannot be used on this host: what is wrong, and the way out, worded for each cause.
struct ContainerUnavailableView: View {
    let runtime: ContainerRuntime
    let reason: DockerUnavailable
    var detail: String?
    /// No runtime of any kind was found (the probe came back empty).
    var noRuntimeAtAll = false
    var retry: () -> Void

    static func content(_ runtime: ContainerRuntime, _ reason: DockerUnavailable, noRuntimeAtAll: Bool) -> (icon: String, title: String, body: String, fix: String?) {
        let name = ContainerRuntimePrefs.title(runtime)
        if noRuntimeAtAll {
            return ("shippingbox", "No container runtime found",
                    "This host has none of docker, podman, nerdctl, kubectl or incus on its PATH.",
                    "Install one of them on the host, or open the tab on the right host.")
        }
        switch (runtime, reason) {
        case (_, .noCluster):
            return ("point.3.connected.trianglepath.dotted", "No cluster selected",
                    "kubectl on this host has no context, so it does not know which cluster to ask.",
                    "On the host: kubectl config use-context <name>. The phone never switches contexts.")
        case (.kubernetes, .daemonDown):
            return ("network.slash", "The cluster is unreachable",
                    "kubectl on the host could not reach the cluster's API server. This is about the host's access, not the phone's.",
                    "Check the VPN or network of the host, then try again.")
        case (.kubernetes, .noPermission):
            return ("lock.shield", "Not allowed to list pods",
                    "The host's kubectl identity is refused by the cluster.",
                    "Ask for read access to pods, or switch to a context that has it (on the host).")
        case (.incus, .noPermission), (.lxd, .noPermission):
            return ("lock.shield", "No permission to use \(name)",
                    "The login user of this host may not talk to the \(name) daemon.",
                    "Linux: add the user to the incus-admin group (lxd group for LXD), then sign in again. That group is root-equivalent, so it is not done for you.")
        case (_, .notInstalled):
            return ("shippingbox", "\(name) is not installed",
                    "There is no \(name.lowercased()) command on this host, so there is nothing to list.",
                    "Install it on the host, or pick another runtime above.")
        case (_, .daemonDown):
            return ("poweroff", "\(name) is not running",
                    "The command is there, but its engine does not answer.",
                    "Start Docker Desktop / OrbStack / Colima (Linux: sudo systemctl start docker, or podman.socket), then try again.")
        case (_, .noPermission):
            return ("lock.shield", "No permission to use \(name)",
                    "The login user of this host may not talk to the \(name) engine.",
                    "Linux: sudo usermod -aG docker $USER, then sign in again. Being in the docker group is root-equivalent, so it is not done for you.")
        case (_, .unexpected):
            return ("questionmark.circle", "\(name) did not answer as expected",
                    "The host replied with something this app cannot read as a list.",
                    "Run the command by hand on the host to see what it says, then try again.")
        }
    }

    var body: some View {
        let c = Self.content(runtime, reason, noRuntimeAtAll: noRuntimeAtAll)
        VStack(spacing: 8) {
            Image(systemName: c.icon).font(.system(size: 30)).foregroundColor(.secondary)
            Text(c.title).font(.headline)
            Text(c.body).font(.caption).foregroundColor(.secondary).multilineTextAlignment(.center)
            if let fix = c.fix { Text(fix).font(.caption).multilineTextAlignment(.center) }
            if reason == .unexpected, let detail { Text(detail).font(.caption2).foregroundColor(.secondary).multilineTextAlignment(.center) }
            Button("Try again", action: retry).buttonStyle(.glass).controlSize(.small)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
    }
}
