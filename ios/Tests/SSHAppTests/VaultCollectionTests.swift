import XCTest
import SshCoreBridge
@testable import SSHApp

final class VaultCollectionTests: XCTestCase {
    private func at(_ s: Double) -> Date { Date(timeIntervalSince1970: s) }
    private func item(_ id: String, _ payload: String) -> CollectionItem { CollectionItem(id: id, payload: payload) }
    private func run(local: [CollectionItem], remote: [VaultEntry] = [], state: CollectionState = CollectionState(), now: Double,
                     accept: (CollectionItem) -> Bool = { _ in true }) -> VaultCollectionEngine.Outcome {
        VaultCollectionEngine.reconcile(prefix: "p_", local: local, remote: remote, state: state, now: at(now), accept: accept)
    }

    func testALocalEditGetsANewTimeAndWinsOverAnOlderRemoteCopy() {
        let first = run(local: [item("a", "v1")], now: 100)
        XCTAssertEqual(first.entries.first { $0.id == "p_a" }?.updatedAtMs, 100_000)
        let old = VaultEntry(id: "p_a", updatedAtMs: 120_000, deleted: false, payload: "theirs")
        let edited = run(local: [item("a", "v2")], remote: [old], state: first.state, now: 200)
        XCTAssertTrue(edited.upserts.isEmpty, "the local edit is newer")
        XCTAssertEqual(edited.entries.first { $0.id == "p_a" }?.payload, "v2")
    }

    func testANewerRemoteCopyReplacesTheLocalOne() {
        let first = run(local: [item("a", "v1")], now: 100)
        let theirs = VaultEntry(id: "p_a", updatedAtMs: 300_000, deleted: false, payload: "v9")
        let out = run(local: [item("a", "v1")], remote: [theirs], state: first.state, now: 200)
        XCTAssertEqual(out.upserts, [item("a", "v9")])
        // The next run with the applied item changes nothing (no echo with a new time).
        let again = run(local: [item("a", "v9")], remote: [theirs], state: out.state, now: 400)
        XCTAssertTrue(again.upserts.isEmpty)
        XCTAssertEqual(again.entries.first { $0.id == "p_a" }?.updatedAtMs, 300_000)
    }

    func testANewItemFromTheOtherDeviceArrivesAndOneThatIsRefusedDoesNot() {
        let theirs = [VaultEntry(id: "p_new", updatedAtMs: 50_000, deleted: false, payload: "ok"),
                      VaultEntry(id: "p_bad", updatedAtMs: 50_000, deleted: false, payload: "evil")]
        let out = run(local: [], remote: theirs, now: 100, accept: { $0.payload == "ok" })
        XCTAssertEqual(out.upserts.map(\.id), ["new"])
        XCTAssertFalse(out.state.known.contains("bad"))
    }

    func testALocalDeletionBecomesATombstoneAndIsNotUndoneByTheOldCopy() {
        let first = run(local: [item("a", "v1")], now: 100)
        let oldCopy = VaultEntry(id: "p_a", updatedAtMs: 100_000, deleted: false, payload: "v1")
        let out = run(local: [], remote: [oldCopy], state: first.state, now: 200)
        XCTAssertTrue(out.upserts.isEmpty)
        XCTAssertEqual(out.entries.first { $0.id == "p_a" }?.deleted, true)
    }

    func testADeletionOnTheOtherDeviceRemovesTheItemHere() {
        let first = run(local: [item("a", "v1")], now: 100)
        let stone = VaultEntry(id: "p_a", updatedAtMs: 300_000, deleted: true, payload: "")
        let out = run(local: [item("a", "v1")], remote: [stone], state: first.state, now: 200)
        XCTAssertEqual(out.deletes, ["a"])
        XCTAssertFalse(out.state.known.contains("a"))
    }

    func testOtherPrefixesAreLeftAlone() {
        let foreign = VaultEntry(id: "s_x", updatedAtMs: 1, deleted: false, payload: "snippet")
        let out = run(local: [], remote: [foreign], now: 100)
        XCTAssertTrue(out.upserts.isEmpty)
        XCTAssertTrue(out.entries.isEmpty)
    }

    // MARK: Profiles

    private func profile(env: [String] = [], hook: String = "phoneApproval") -> SavedAgentProfile {
        var p = SavedAgentProfile(name: "Claude", agent: .claudeCode, binary: "claude", env: env)
        p.hookKindRaw = hook
        return p
    }

    func testSecretsAndCustomHooksNeverLeaveTheDevice() {
        let p = profile(env: ["ANTHROPIC_API_KEY=sk-123", "HTTPS_PROXY=http://p:1", "GH_TOKEN=x", "MY_PASSWORD=y"])
        let out = SyncedProfile.outgoing([p, profile(hook: "custom")])
        XCTAssertEqual(out.count, 1, "the custom-hook profile stays local")
        XCTAssertFalse(out[0].payload.contains("sk-123"))
        XCTAssertFalse(out[0].payload.contains("GH_TOKEN"))
        XCTAssertTrue(out[0].payload.contains("HTTPS_PROXY"))
    }

    func testAReceivedProfileKeepsTheLocalSecretsAndCustomHook() {
        var local = profile(env: ["API_KEY=mine", "A=1"])
        local.customHookCommand = "my-hook"
        var incoming = local
        incoming.env = ["A=2"]
        incoming.customHookCommand = ""
        let merged = SyncedProfile.merged(incoming, into: local)
        XCTAssertEqual(merged.env.sorted(), ["A=2", "API_KEY=mine"])
        XCTAssertEqual(merged.customHookCommand, "my-hook")
    }

    func testHostileProfilesAreRefused() {
        func decode(_ edit: (inout SavedAgentProfile) -> Void) -> SavedAgentProfile? {
            var p = profile()
            edit(&p)
            let data = try! JSONEncoder().encode(p)
            return SyncedProfile.decode(CollectionItem(id: p.id, payload: String(data: data, encoding: .utf8)!))
        }
        XCTAssertNotNil(decode { _ in })
        XCTAssertNil(decode { $0.binary = "-rf" })
        XCTAssertNil(decode { $0.binary = "a\nb" })
        XCTAssertNil(decode { $0.args = ["ok", "x\u{0}y"] })
        XCTAssertNil(decode { $0.env = ["NOEQUALS"] })
        XCTAssertNil(decode { $0.env = ["A B=1"] })
        XCTAssertNil(decode { $0.env = ["SECRET_TOKEN=1"] })
        XCTAssertNil(decode { $0.hookKindRaw = "custom" })
        XCTAssertNil(decode { $0.args = Array(repeating: "x", count: 65) })
        XCTAssertNil(decode { $0.name = String(repeating: "n", count: 101) })
        let p = profile()
        XCTAssertNil(SyncedProfile.decode(CollectionItem(id: "other-id", payload: String(data: try! JSONEncoder().encode(p), encoding: .utf8)!)),
                     "the id inside must be the id of the entry")
    }

    // MARK: Snippets

    func testSnippetsWithASecretStayLocalAndHostileOnesAreRefused() {
        let ok = SyncedSnippet(id: "custom-1", title: "List", command: "ls -la", category: "Custom", tags: ["custom"])
        let secret = SyncedSnippet(id: "custom-2", title: "Login", command: "export API_KEY=abc", category: "Custom", tags: [])
        let bearer = SyncedSnippet(id: "custom-3", title: "Curl", command: "curl -H 'Authorization: Bearer abc'", category: "Custom", tags: [])
        XCTAssertEqual(SyncedSnippet.outgoing([ok, secret, bearer]).map(\.id), ["custom-1"])
        let wire = SyncedSnippet.outgoing([ok])[0]
        XCTAssertEqual(SyncedSnippet.decode(wire), ok)
        var long = ok; long.command = String(repeating: "x", count: 4001)
        XCTAssertNil(SyncedSnippet.decode(CollectionItem(id: long.id, payload: String(data: try! JSONEncoder().encode(long), encoding: .utf8)!)))
        var nul = ok; nul.title = "a\u{0}b"
        XCTAssertNil(SyncedSnippet.decode(CollectionItem(id: nul.id, payload: String(data: try! JSONEncoder().encode(nul), encoding: .utf8)!)))
        XCTAssertNil(SyncedSnippet.decode(CollectionItem(id: "x", payload: String(data: try! JSONEncoder().encode(ok), encoding: .utf8)!)))
    }
}
