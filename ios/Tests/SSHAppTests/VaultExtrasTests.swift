import XCTest
@testable import SSHApp

final class VaultExtrasTests: XCTestCase {
    func testEqualButtonsShareOneIDAndOneItem() {
        let a = QuickCommand(label: "tests", text: "make test")
        let b = QuickCommand(label: "tests", text: "make test")
        XCTAssertNotEqual(a.id, b.id)
        XCTAssertEqual(SyncedQuickCommand(a).id, SyncedQuickCommand(b).id)
        XCTAssertEqual(SyncedQuickCommand.outgoing([a, b]).count, 1)
    }

    func testEditedButtonIsANewID() {
        XCTAssertNotEqual(SyncedQuickCommand(QuickCommand(label: "x", text: "ls")).id,
                          SyncedQuickCommand(QuickCommand(label: "x", text: "ls -la")).id)
    }

    func testSecretLookingButtonStaysOnTheDevice() {
        XCTAssertTrue(SyncedQuickCommand.outgoing([QuickCommand(label: "k", text: "export API_KEY=abc")]).isEmpty)
    }

    func testRoundTripAndHostileDecode() throws {
        let c = QuickCommand(label: "model", text: "/model", menu: QuickCommandMenu(source: .fixed, template: "/model {choice}", items: "a\nb"))
        let item = try XCTUnwrap(SyncedQuickCommand.outgoing([c]).first)
        let back = try XCTUnwrap(SyncedQuickCommand.decode(item))
        XCTAssertEqual(back.command.text, "/model")
        XCTAssertEqual(back.command.menu?.items, "a\nb")
        // Another id than the content's, a control character, a secret and a huge text are refused.
        XCTAssertNil(SyncedQuickCommand.decode(CollectionItem(id: "wrong", payload: item.payload)))
        var evil = SyncedQuickCommand(QuickCommand(label: "x", text: "ls\u{1b}[2J"))
        XCTAssertNil(SyncedQuickCommand.decode(CollectionItem(id: evil.id, payload: json(evil))))
        evil = SyncedQuickCommand(QuickCommand(label: "x", text: String(repeating: "a", count: 1001)))
        XCTAssertNil(SyncedQuickCommand.decode(CollectionItem(id: evil.id, payload: json(evil))))
        evil = SyncedQuickCommand(QuickCommand(label: "x", text: "TOKEN=abc ls"))
        XCTAssertNil(SyncedQuickCommand.decode(CollectionItem(id: evil.id, payload: json(evil))))
        XCTAssertNil(SyncedQuickCommand.decode(CollectionItem(id: "q", payload: "{bad")))
    }

    private func json(_ s: SyncedQuickCommand) -> String {
        String(data: (try? JSONEncoder().encode(s)) ?? Data(), encoding: .utf8) ?? ""
    }

    func testAccentOnlyWhenChosenAndOnlyValidValues() {
        let d = UserDefaults(suiteName: "vault-extras-test")!
        d.removePersistentDomain(forName: "vault-extras-test")
        XCTAssertTrue(SyncedAppearance.outgoing(defaults: d).isEmpty)
        d.set("teal", forKey: AppAccent.storageKey)
        let item = SyncedAppearance.outgoing(defaults: d)
        XCTAssertEqual(item.count, 1)
        XCTAssertEqual(SyncedAppearance.decode(item[0]), "teal")
        d.set("#12ab9F", forKey: AppAccent.storageKey)
        XCTAssertEqual(SyncedAppearance.outgoing(defaults: d).count, 1)
        d.set("javascript:x", forKey: AppAccent.storageKey)
        XCTAssertTrue(SyncedAppearance.outgoing(defaults: d).isEmpty)
        XCTAssertNil(SyncedAppearance.decode(CollectionItem(id: "accent", payload: ##"{"accent":"#12345"}"##)))
        XCTAssertNil(SyncedAppearance.decode(CollectionItem(id: "other", payload: #"{"accent":"teal"}"#)))
    }
}
