import XCTest
import SwiftUI
@testable import SSHApp

final class AppearanceTests: XCTestCase {
    private let key = "terminal_custom_schemes"

    override func setUp() {
        UserDefaults.standard.removeObject(forKey: key)
        UserDefaults.standard.removeObject(forKey: SchemeSlot.claude.storageKey)
    }

    override func tearDown() { setUp() }

    func testBuiltInSchemesAreWellFormed() {
        let ids = TerminalScheme.builtIn.map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count)
        for s in TerminalScheme.builtIn {
            XCTAssertEqual(s.ansi.count, 16, s.id)
            for h in [s.background, s.foreground, s.cursor] + s.ansi {
                XCTAssertEqual(h.count, 6, "\(s.id) \(h)")
                XCTAssertNotNil(UInt32(h, radix: 16), "\(s.id) \(h)")
            }
        }
    }

    func testCustomSchemeJSONRoundTripAndDelete() {
        let c = TerminalScheme.newCustom(from: .fallback, name: "Mine")
        XCTAssertTrue(c.isCustom)
        TerminalScheme.upsertCustom(c)
        XCTAssertEqual(TerminalScheme.scheme(id: c.id), c)
        var edited = c
        edited.name = "Renamed"
        TerminalScheme.upsertCustom(edited)
        XCTAssertEqual(TerminalScheme.custom.count, 1)
        XCTAssertEqual(TerminalScheme.custom.first?.name, "Renamed")
        TerminalScheme.deleteCustom(id: c.id)
        XCTAssertNil(TerminalScheme.scheme(id: c.id))
    }

    func testBrokenCustomSchemeIsDropped() throws {
        var bad = TerminalScheme.newCustom(from: .fallback, name: "Bad")
        bad.ansi = ["000000"]
        UserDefaults.standard.set(try JSONEncoder().encode([bad]), forKey: key)
        XCTAssertTrue(TerminalScheme.custom.isEmpty)
    }

    func testResolveOrder() {
        XCTAssertEqual(TerminalSchemes.resolve(profileSchemeId: nil, slot: .claude).id, "gruvbox")
        XCTAssertEqual(TerminalSchemes.resolve(profileSchemeId: "nord", slot: .claude).id, "nord")
        XCTAssertEqual(TerminalSchemes.resolve(profileSchemeId: "gone", slot: .claude).id, "gruvbox")
        TerminalSchemes.setChoice("dracula", for: .claude)
        XCTAssertEqual(TerminalSchemes.resolve(profileSchemeId: nil, slot: .claude).id, "dracula")
    }

    func testDeletedCustomSchemeInSlotFallsBackToBuiltInChoice() {
        let c = TerminalScheme.newCustom(from: .fallback, name: "Tmp")
        TerminalScheme.upsertCustom(c)
        TerminalSchemes.setChoice(c.id, for: .claude)
        TerminalScheme.deleteCustom(id: c.id)
        XCTAssertEqual(TerminalSchemes.choice(for: .claude), "gruvbox")
    }

    func testAccentHexAndPresets() {
        XCTAssertEqual(Color(hex: "ff8000").hexString, "ff8000")
        XCTAssertEqual(Color(hex: "zzzzzz").hexString, "000000")
        XCTAssertEqual(AppAccent.color(for: "#336699").hexString, "336699")
        XCTAssertEqual(AppAccent.color(for: "orange").hexString, AppAccent.orange.color.hexString)
        XCTAssertEqual(AppAccent.color(for: "nonsense").hexString, AppAccent.blue.color.hexString)
        XCTAssertEqual(AppAccent.color(for: "#12345").hexString, AppAccent.blue.color.hexString)
    }

    func testIconNamesRoundTrip() {
        XCTAssertEqual(AppIconChoice.standard.alternateName, nil)
        for name in AppIconChoice.allAlternateNames {
            XCTAssertEqual(AppIconChoice.parse(alternateName: name).alternateName, name, name)
        }
        XCTAssertEqual(AppIconChoice.parse(alternateName: "garbage"), .standard)
        XCTAssertEqual(Set(AppIconChoice.allAlternateNames).count, AppIconChoice.allAlternateNames.count)
    }

    func testProfileIconReferences() {
        XCTAssertTrue(ProfileIcons.isValid("glyph:>_"))
        XCTAssertFalse(ProfileIcons.isValid("glyph:"))
        XCTAssertFalse(ProfileIcons.isValid("img:../../etc/passwd"))
        XCTAssertFalse(ProfileIcons.isValid("img:missing.png"))
        XCTAssertFalse(ProfileIcons.isValid(""))
    }
}
