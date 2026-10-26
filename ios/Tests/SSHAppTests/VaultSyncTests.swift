import XCTest
import SshCoreBridge
@testable import SSHApp

final class VaultSyncTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_000)

    private func profile(_ name: String, host: String = "h.example", at seconds: Double, fp: String? = nil, key: String? = nil) -> ServerProfile {
        ServerProfile(name: name, host: host, port: 22, username: "me", hostKeyFingerprint: fp, keyId: key,
                      updatedAt: Date(timeIntervalSince1970: seconds))
    }

    private func remote(of profiles: [ServerProfile], fingerprints: Bool = false) -> [VaultEntry] {
        VaultSyncEngine.entries(for: profiles, tombstones: [:], syncFingerprints: fingerprints)
    }

    func testANewServerFromTheOtherDeviceArrivesWithoutKeyOrDefault() {
        let theirs = profile("Mac", at: 100, key: "their-key")
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(out.profiles.count, 1)
        XCTAssertEqual(out.profiles[0].id, theirs.id)
        XCTAssertNil(out.profiles[0].keyId, "each device signs with its own key")
        XCTAssertFalse(out.profiles[0].isDefault)
        XCTAssertTrue(out.profilesChanged)
    }

    func testTheLaterEditWinsAndKeepsLocalKeyChoice() {
        var mine = profile("Old name", at: 100, key: "my-key")
        var theirs = mine
        theirs.name = "New name"; theirs.updatedAt = Date(timeIntervalSince1970: 200)
        let out = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(out.profiles[0].name, "New name")
        XCTAssertEqual(out.profiles[0].keyId, "my-key")
        mine.updatedAt = Date(timeIntervalSince1970: 300)
        let kept = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now)
        XCTAssertEqual(kept.profiles[0].name, "Old name")
        XCTAssertFalse(kept.profilesChanged)
    }

    func testADeletionOnTheOtherDeviceRemovesTheServerHere() {
        let p = profile("Gone", at: 100)
        let stone = VaultEntry(id: p.id.uuidString, updatedAtMs: 200_000, deleted: true, payload: "")
        let out = VaultSyncEngine.reconcile(local: [p], tombstones: [:], remote: [stone], syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
        XCTAssertEqual(out.tombstones[p.id.uuidString], 200_000)
    }

    func testALocalDeletionIsNotUndoneByAnOlderRemoteCopy() {
        let p = profile("Gone", at: 100)
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [p.id.uuidString: 300_000], remote: remote(of: [p]), syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
    }

    func testFingerprintsTravelOnlyWithTheirToggleAndADifferentOneIsAConflict() {
        let theirs = profile("Mac", at: 200, fp: "SHA256:theirs")
        XCTAssertNil(VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: remote(of: [theirs]), syncFingerprints: false, now: now).profiles.first?.hostKeyFingerprint)
        XCTAssertFalse(remote(of: [theirs]).first!.payload.contains("SHA256"))

        let onWire = remote(of: [theirs], fingerprints: true)
        let fresh = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: onWire, syncFingerprints: true, now: now)
        XCTAssertEqual(fresh.profiles[0].hostKeyFingerprint, "SHA256:theirs")

        var mine = theirs; mine.hostKeyFingerprint = "SHA256:mine"; mine.updatedAt = Date(timeIntervalSince1970: 100)
        let clash = VaultSyncEngine.reconcile(local: [mine], tombstones: [:], remote: onWire, syncFingerprints: true, now: now)
        XCTAssertEqual(clash.profiles[0].hostKeyFingerprint, "SHA256:mine", "a pin is never replaced silently")
        XCTAssertEqual(clash.fingerprintConflicts, ["Mac"])
    }

    func testHostileRemotePayloadsNeverBecomeProfiles() {
        let bad = [
            #"{"name":"x","host":"-oProxyCommand=evil","port":22,"username":"me"}"#,
            #"{"name":"x","host":"a b","port":22,"username":"me"}"#,
            #"{"name":"x","host":"h","port":0,"username":"me"}"#,
            #"{"name":"x","host":"h","port":22,"username":"-l root"}"#,
            "not json",
        ]
        let entries = bad.map { VaultEntry(id: UUID().uuidString, updatedAtMs: 5, deleted: false, payload: $0) }
        let out = VaultSyncEngine.reconcile(local: [], tombstones: [:], remote: entries, syncFingerprints: false, now: now)
        XCTAssertTrue(out.profiles.isEmpty)
    }

    func testThePayloadCarriesNoKeyChoiceOrDefaultFlag() {
        let p = profile("Mac", at: 100, key: "secret-key-id")
        let wire = remote(of: [p])
        XCTAssertEqual(wire.count, 1)
        XCTAssertFalse(wire[0].payload.contains("secret-key-id"))
        XCTAssertFalse(wire[0].payload.contains("isDefault"))
    }
}

/// The package test host has no Keychain entitlement (error -34018), so tests that store keys skip
/// there; the store logic itself is covered by the core's tests with an in-memory backend, and the
/// Keychain backend by the device check in docs/todo.md.
func requireKeychain() throws {
    let probe = KeyRef(id: UUID().uuidString, guarded: false)
    defer { try? keyDelete(key: probe) }
    do { _ = try keyGenerate(key: probe, overwrite: false) } catch {
        throw XCTSkip("No Keychain access in this test host: \(error)")
    }
}

final class KeyImportTests: XCTestCase {
    func testImportTakesBytesAndNeverHandsTheKeyBack() throws {
        try requireKeychain()
        let a = KeyRef(id: UUID().uuidString, guarded: false), b = KeyRef(id: UUID().uuidString, guarded: false)
        defer { try? keyDelete(key: a); try? keyDelete(key: b) }
        let made = try keyGenerate(key: a, overwrite: false)
        let acks = ExportAcks(understandsAccess: true, understandsExposure: true, typedConfirmation: "EXPORT")
        var file = try keyExportPlain(key: a, acks: acks)
        defer { file.resetBytes(in: 0..<file.count) }
        XCTAssertFalse(try keyFileNeedsPassphrase(armored: file))
        XCTAssertEqual(try keyImport(key: b, armored: file, passphrase: nil, overwrite: false).fingerprintSha256, made.fingerprintSha256)
        XCTAssertThrowsError(try keyImport(key: b, armored: Data("garbage".utf8), passphrase: nil, overwrite: true))
    }

    func testExportNeedsAllThreeConfirmations() throws {
        try requireKeychain()
        let a = KeyRef(id: UUID().uuidString, guarded: false)
        defer { try? keyDelete(key: a) }
        try keyGenerate(key: a, overwrite: false)
        XCTAssertThrowsError(try keyExportPlain(key: a, acks: ExportAcks(understandsAccess: true, understandsExposure: false, typedConfirmation: "EXPORT")))
        XCTAssertThrowsError(try keyExportPlain(key: a, acks: ExportAcks(understandsAccess: true, understandsExposure: true, typedConfirmation: "export")))
    }
}

final class HostScreensTests: XCTestCase {
    func testHostScreensAreOwnQuickMenuTilesThatNeedAConnection() {
        for a in [AppAction.telemetry, .processes, .docker, .hostHelper] {
            XCTAssertTrue(a.requiresConnection)
            XCTAssertTrue(a.canHide)
            XCTAssertFalse(a.isSessionAction)
        }
        // An order saved before these tiles existed still lists them, at the end.
        let old = QuickMenuPrefs.raw(from: [.settings, .telemetry])
        let order = QuickMenuPrefs.ordered(from: old)
        XCTAssertEqual(Array(order.prefix(2)), [.settings, .telemetry])
        XCTAssertTrue(order.contains(.docker) && order.contains(.processes) && order.contains(.hostHelper))
    }

    func testEachUnavailableReasonHasItsOwnWording() {
        let snap = parseDockerList(output: "DOCKER_NOT_INSTALLED\n")
        XCTAssertEqual(snap.unavailable, .notInstalled)
        XCTAssertEqual(parseDockerList(output: "permission denied while trying to connect to the Docker daemon socket").unavailable, .noPermission)
        XCTAssertEqual(parseDockerList(output: "Cannot connect to the Docker daemon. Is the docker daemon running?").unavailable, .daemonDown)
        XCTAssertEqual(parseDockerList(output: "").unavailable, nil)
    }
}

final class ContainersScreenTests: XCTestCase {
    private func info(_ id: String, project: String? = nil, scope: String = "", restartable: Bool = true) -> DockerContainerInfo {
        DockerContainerInfo(id: id, name: id, image: "img", state: .running, statusText: "Up", createdAt: "", ports: "",
                            unhealthy: false, project: project, service: nil, scope: scope, restartable: restartable)
    }

    func testEnginesGroupByComposeProjectWithLooseContainersLast() {
        let groups = ContainerGroup.make([info("a1", project: "shop"), info("b1"), info("a2", project: "shop"), info("c1", project: "blog")], runtime: .podman)
        XCTAssertEqual(groups.map(\.title), ["blog", "shop", "Not in a stack"])
        XCTAssertEqual(groups[1].items.map(\.id), ["a1", "a2"])
        XCTAssertNotNil(groups[1].project)
        XCTAssertNil(groups[2].project, "loose containers have no stack actions")
    }

    func testOnlyLooseContainersAreOneListWithoutAHeader() {
        let groups = ContainerGroup.make([info("x1"), info("x2")], runtime: .docker)
        XCTAssertEqual(groups.count, 1)
        XCTAssertEqual(groups[0].title, "")
    }

    func testKubernetesGroupsByNamespaceAndIncusIsOneList() {
        let pods = [info("b/p1", scope: "b"), info("a/p2", scope: "a"), info("a/p3", scope: "a")]
        XCTAssertEqual(ContainerGroup.make(pods, runtime: .kubernetes).map(\.title), ["a", "b"])
        XCTAssertNil(ContainerGroup.make(pods, runtime: .kubernetes)[0].project, "no stack actions on a namespace")
        XCTAssertEqual(ContainerGroup.make(pods, runtime: .incus).count, 1)
    }

    func testCapabilitiesDecideWhatIsOffered() {
        XCTAssertFalse(runtimeCaps(runtime: .kubernetes).startStop)
        XCTAssertFalse(runtimeCaps(runtime: .kubernetes).stats)
        XCTAssertTrue(runtimeCaps(runtime: .podman).stats)
        XCTAssertFalse(runtimeCaps(runtime: .incus).groups)
    }

    func testEveryReasonHasWordsForEveryRuntime() {
        let runtimes: [ContainerRuntime] = [.docker, .podman, .nerdctl, .kubernetes, .incus, .lxd]
        let reasons: [DockerUnavailable] = [.notInstalled, .noPermission, .daemonDown, .unexpected, .noCluster]
        for r in runtimes {
            for why in reasons {
                let c = ContainerUnavailableView.content(r, why, noRuntimeAtAll: false)
                XCTAssertFalse(c.title.isEmpty); XCTAssertFalse(c.body.isEmpty); XCTAssertNotNil(c.fix)
            }
        }
        XCTAssertTrue(ContainerUnavailableView.content(.kubernetes, .daemonDown, noRuntimeAtAll: false).title.contains("cluster"))
        XCTAssertTrue(ContainerUnavailableView.content(.docker, .notInstalled, noRuntimeAtAll: true).title.contains("No container runtime"))
        XCTAssertTrue(ContainerUnavailableView.content(.incus, .noPermission, noRuntimeAtAll: false).body.contains("Incus"))
    }

    func testAStackMessageSaysWhatIsDoneFailedAndNotStarted() {
        let names = ["a": "web", "b": "db", "c": "cache"]
        let partial = StackOutcome(done: ["a"], failedId: "b", failedReason: "no such container", notStarted: ["c"], unknown: false)
        let text = StackMessage.text(partial, names: names)
        XCTAssertTrue(text.contains("Done: 1") && text.contains("Failed: db (no such container)") && text.contains("Not started: cache"))
        let cut = StackOutcome(done: ["a"], failedId: nil, failedReason: nil, notStarted: ["b"], unknown: true)
        XCTAssertTrue(StackMessage.text(cut, names: names).contains("may be partly done"))
        let ok = StackOutcome(done: ["a", "b"], failedId: nil, failedReason: nil, notStarted: [], unknown: false)
        XCTAssertEqual(StackMessage.text(ok, names: names), "Done: 2.")
    }

    func testRuntimeChoiceAndVerificationArePerServer() {
        let server = UUID(), other = UUID()
        ContainerRuntimePrefs.choose(.podman, server: server)
        XCTAssertEqual(ContainerRuntimePrefs.chosen(server: server), .podman)
        XCTAssertNil(ContainerRuntimePrefs.chosen(server: other))
        XCTAssertTrue(ContainerRuntimePrefs.isVerified(.docker, server: server), "Docker is the one seen on a real host")
        XCTAssertFalse(ContainerRuntimePrefs.isVerified(.podman, server: server))
        ContainerRuntimePrefs.markVerified(.podman, server: server)
        XCTAssertTrue(ContainerRuntimePrefs.isVerified(.podman, server: server))
        XCTAssertFalse(ContainerRuntimePrefs.isVerified(.podman, server: other))
        for r in [ContainerRuntime.docker, .podman, .nerdctl, .kubernetes, .incus, .lxd] {
            XCTAssertEqual(ContainerRuntimePrefs.parse(ContainerRuntimePrefs.name(r)), r)
        }
    }

    func testTheCoreBuildsShellAndSessionNamesPerRuntime() throws {
        let pod = try buildRuntimeExecCommand(runtime: .kubernetes, containerId: "app/api-7d9")
        XCTAssertTrue(pod.contains("kubectl exec -it -n 'app' 'api-7d9'"))
        XCTAssertThrowsError(try buildRuntimeExecCommand(runtime: .kubernetes, containerId: "app/api; id"))
        let name = try runtimeSessionName(runtime: .podman, containerName: "web", containerId: "3f2a9c1d77")
        XCTAssertTrue(name.hasPrefix("podman-web-"))
    }
}
