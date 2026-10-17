import XCTest
@testable import SSHApp

final class SubscriptionTests: XCTestCase {
    func testFreeTierLimits() {
        XCTAssertTrue(FreeTier.canAddSession(open: 19, isPro: false))
        XCTAssertFalse(FreeTier.canAddSession(open: 20, isPro: false))
        XCTAssertTrue(FreeTier.canAddSession(open: 500, isPro: true))
        XCTAssertTrue(FreeTier.canAddServer(count: 2, isPro: false))
        XCTAssertFalse(FreeTier.canAddServer(count: 3, isPro: false))
        XCTAssertTrue(FreeTier.canAddServer(count: 30, isPro: true))
    }

    func testLimitsApplyOnlyWhenPaymentsOnAndNotPro() {
        XCTAssertTrue(Monetization.limitsApply(enabled: true, isPro: false))
        XCTAssertFalse(Monetization.limitsApply(enabled: true, isPro: true))
        XCTAssertFalse(Monetization.limitsApply(enabled: false, isPro: false))
    }

    func testEntitlementGrants() {
        let now = Date()
        let hour: TimeInterval = 3600
        let weekly = ProPlan.weekly.rawValue
        XCTAssertTrue(EntitlementSnapshot(productId: weekly, expirationDate: now + hour, revocationDate: nil).grantsPro(now: now))
        XCTAssertFalse(EntitlementSnapshot(productId: weekly, expirationDate: now - hour, revocationDate: nil).grantsPro(now: now))
        XCTAssertFalse(EntitlementSnapshot(productId: weekly, expirationDate: now + hour, revocationDate: now).grantsPro(now: now))
        XCTAssertTrue(EntitlementSnapshot(productId: ProPlan.lifetime.rawValue, expirationDate: nil, revocationDate: nil).grantsPro(now: now))
        XCTAssertFalse(EntitlementSnapshot(productId: ProPlan.lifetime.rawValue, expirationDate: nil, revocationDate: now).grantsPro(now: now))
        XCTAssertFalse(EntitlementSnapshot(productId: "com.other.app", expirationDate: nil, revocationDate: nil).grantsPro(now: now))
    }

    func testPlans() {
        XCTAssertEqual(ProPlan.allCases.count, 4)
        XCTAssertEqual(Set(ProPlan.allIds).count, 4)
        XCTAssertFalse(ProPlan.lifetime.isSubscription)
        XCTAssertTrue(ProPlan.yearly.isSubscription)
    }
}
