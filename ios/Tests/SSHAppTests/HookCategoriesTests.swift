import XCTest
@testable import SSHApp

final class HookCategoriesTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let d = UserDefaults(suiteName: "hookcat-\(UUID().uuidString)")!
        return d
    }

    func testALabelIsCleanedAndOtherIsNotACategory() {
        XCTAssertEqual(HookCategories.normalized("  work\n "), "work")
        XCTAssertNil(HookCategories.normalized("   "))
        XCTAssertNil(HookCategories.normalized("other"))
        XCTAssertEqual(HookCategories.normalized(String(repeating: "x", count: 60))?.count, HookCategories.maxLength)
    }

    func testFoldersGroupByCategoryPerServerWithOtherLast() {
        let d = defaults()
        let a = UUID(), b = UUID()
        HookCategories.set("work", server: a, folder: "/p/api", d)
        HookCategories.set("play", server: a, folder: "/p/toy", d)
        HookCategories.set("work", server: b, folder: "/p/other-server", d)
        let map = HookCategories.all(d)
        let g = HookCategories.grouped(["/p/api", "/p/misc", "/p/toy", "/p/web"], server: a, in: map)
        XCTAssertEqual(g.map(\.category), ["play", "work", "Other"])
        XCTAssertEqual(g.last?.folders, ["/p/misc", "/p/web"])
        // The label of another server does not leak into this one.
        XCTAssertEqual(HookCategories.category(server: a, folder: "/p/other-server", in: map), "Other")
    }

    func testAnEmptyLabelRemovesTheCategory() {
        let d = defaults()
        let s = UUID()
        HookCategories.set("work", server: s, folder: "/p/api", d)
        HookCategories.set("", server: s, folder: "/p/api", d)
        XCTAssertEqual(HookCategories.category(server: s, folder: "/p/api", in: HookCategories.all(d)), "Other")
    }
}
