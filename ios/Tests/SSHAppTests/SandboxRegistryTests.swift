import XCTest
@testable import SSHApp

final class SandboxRegistryTests: XCTestCase {
    private func defaults() -> UserDefaults { UserDefaults(suiteName: "sbxreg-\(UUID().uuidString)")! }

    private func record(_ id: String, server: String? = nil, age: TimeInterval = 0, agent: String = "claude") -> SandboxRecord {
        SandboxRecord(id: id, agentName: agent, folder: "~/work/api", engineName: "docker", online: true,
                      autoApprove: false, copyLogin: true, claudeConfigDir: "", serverId: server,
                      label: "Claude Code", created: Date(timeIntervalSince1970: 1_000 + age))
    }

    func testSeveralSandboxesLiveSideBySideAndOneIsRemovedAlone() {
        let d = defaults()
        SandboxRegistry.upsert(record("sbx-aaaa1111"), d)
        SandboxRegistry.upsert(record("sbx-bbbb2222", age: 5), d)
        XCTAssertEqual(SandboxRegistry.all(d).map(\.id), ["sbx-aaaa1111", "sbx-bbbb2222"])
        SandboxRegistry.remove("sbx-aaaa1111", d)
        XCTAssertEqual(SandboxRegistry.all(d).map(\.id), ["sbx-bbbb2222"])
    }

    func testAnUpsertReplacesTheSameId() {
        let d = defaults()
        SandboxRegistry.upsert(record("sbx-aaaa1111"), d)
        var changed = record("sbx-aaaa1111")
        changed.online = false
        SandboxRegistry.upsert(changed, d)
        XCTAssertEqual(SandboxRegistry.all(d).count, 1)
        XCTAssertFalse(SandboxRegistry.all(d)[0].online)
    }

    func testTheListIsCappedAndTheOldestFallOff() {
        let d = defaults()
        for i in 0..<(SandboxRegistry.maxEntries + 3) { SandboxRegistry.upsert(record("sbx-\(String(format: "%08d", i))", age: Double(i)), d) }
        let all = SandboxRegistry.all(d)
        XCTAssertEqual(all.count, SandboxRegistry.maxEntries)
        XCTAssertEqual(all.first?.id, "sbx-00000003")
    }

    func testAServerSeesItsOwnSandboxesNewestFirst() {
        let s = UUID()
        let list = [record("sbx-aaaa1111", server: s.uuidString, age: 1),
                    record("sbx-bbbb2222", server: UUID().uuidString, age: 2),
                    record("sbx-cccc3333", server: s.uuidString, age: 3)]
        XCTAssertEqual(SandboxRegistry.records(on: s, in: list).map(\.id), ["sbx-cccc3333", "sbx-aaaa1111"])
        let withDefault = list + [record("sbx-dddd4444", server: nil, age: 4)]
        XCTAssertEqual(SandboxRegistry.records(on: nil, in: withDefault).map(\.id), ["sbx-dddd4444"])
        XCTAssertFalse(SandboxRegistry.records(on: s, in: withDefault).contains { $0.id == "sbx-dddd4444" })
    }

    func testTheProfileCarriesTheRecordsChoices() {
        let p = record("sbx-aaaa1111", agent: "codex").profile(allowHosts: ["a.com"])
        XCTAssertEqual(p.sandboxId, "sbx-aaaa1111")
        XCTAssertEqual(p.allowHosts, ["a.com"])
        XCTAssertEqual(p.agent, .codex)
    }
}
