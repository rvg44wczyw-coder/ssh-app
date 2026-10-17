import Foundation
import StoreKit

/// The products sold in the app. Ids must match App Store Connect (and `SSHApp.storekit` for local tests).
public enum ProPlan: String, CaseIterable, Identifiable, Sendable {
    case weekly = "com.semenov.sshapp.pro.weekly"
    case monthly = "com.semenov.sshapp.pro.monthly"
    case yearly = "com.semenov.sshapp.pro.yearly"
    case lifetime = "com.semenov.sshapp.pro.lifetime"

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .weekly: return "Weekly"
        case .monthly: return "Monthly"
        case .yearly: return "Yearly"
        case .lifetime: return "Lifetime"
        }
    }

    /// "per week" etc.; the lifetime plan is a single payment.
    public var periodNote: String {
        switch self {
        case .weekly: return "per week"
        case .monthly: return "per month"
        case .yearly: return "per year"
        case .lifetime: return "one-time purchase"
        }
    }

    public var isSubscription: Bool { self != .lifetime }

    public static let allIds: [String] = allCases.map(\.rawValue)
}

/// The payment system as a whole can be switched off. It is a build option: the `PAYMENTS`
/// compilation condition (Build Settings → Swift Compiler → Active Compilation Conditions).
/// Without it there are no limits, no paywall and no Subscription row in Settings. Debug builds
/// can also flip it at runtime (Settings → Subscription).
public enum Monetization {
    public static let storageKey = "payments_enabled"

    public static var compiledIn: Bool {
        #if PAYMENTS
        return true
        #else
        return false
        #endif
    }

    public static var enabled: Bool {
        guard compiledIn else { return false }
        #if DEBUG
        return UserDefaults.standard.object(forKey: storageKey) as? Bool ?? true
        #else
        return true
        #endif
    }

    /// Whether the limits apply for a user who has (or has not) paid.
    public static func limitsApply(enabled: Bool, isPro: Bool) -> Bool { enabled && !isPro }
}

/// What the app does without a subscription. Existing items above a limit are kept; only adding
/// a new one is refused.
public enum FreeTier {
    public static let maxSessions = 20
    public static let maxServers = 3

    public static func canAddSession(open: Int, isPro: Bool) -> Bool { isPro || open < maxSessions }
    public static func canAddServer(count: Int, isPro: Bool) -> Bool { isPro || count < maxServers }
}

/// What a set of verified transactions grants. Pure, so it can be tested without the App Store.
public struct EntitlementSnapshot: Equatable, Sendable {
    public let productId: String
    public let expirationDate: Date?
    public let revocationDate: Date?

    public init(productId: String, expirationDate: Date?, revocationDate: Date?) {
        self.productId = productId
        self.expirationDate = expirationDate
        self.revocationDate = revocationDate
    }

    public func grantsPro(now: Date = Date()) -> Bool {
        guard ProPlan(rawValue: productId) != nil, revocationDate == nil else { return false }
        if let end = expirationDate { return end > now }
        return true
    }
}

/// StoreKit 2: loads the products, buys, restores and keeps `isPro` current. Verification is
/// done on the device (signed transactions); there is no server.
@MainActor
public final class SubscriptionManager: ObservableObject {
    public static let shared = SubscriptionManager()

    @Published public private(set) var isPro = false
    /// Payment system on (see `Monetization`). When off, `unlocked` is always true.
    @Published public private(set) var paymentsEnabled = Monetization.enabled
    /// Sorted weekly, monthly, yearly, lifetime.
    @Published public private(set) var products: [Product] = []
    /// Product ids for which the introductory (free trial) offer still applies to this Apple ID.
    @Published public private(set) var trialEligible: Set<String> = []
    @Published public private(set) var isBusy = false
    @Published public private(set) var loadFailed = false
    @Published public var message: String?
    /// The plan that currently grants access (for the settings row).
    @Published public private(set) var activePlan: ProPlan?

    private var updatesTask: Task<Void, Never>?

    /// Free to use everything: payments are off, or Pro is active.
    public var unlocked: Bool { !Monetization.limitsApply(enabled: paymentsEnabled, isPro: isPro) }

    #if DEBUG
    public func setPaymentsEnabled(_ on: Bool) {
        UserDefaults.standard.set(on, forKey: Monetization.storageKey)
        paymentsEnabled = Monetization.enabled
    }
    #endif

    private init() {
        updatesTask = Task { [weak self] in
            for await update in Transaction.updates {
                guard let self else { return }
                if case .verified(let transaction) = update { await transaction.finish() }
                await self.refreshEntitlements()
            }
        }
        Task {
            await refreshEntitlements()
            await loadProducts()
        }
    }

    public func loadProducts() async {
        do {
            let loaded = try await Product.products(for: ProPlan.allIds)
            products = loaded.sorted { rank($0.id) < rank($1.id) }
            loadFailed = products.isEmpty
            var eligible = Set<String>()
            for product in products {
                if let sub = product.subscription, sub.introductoryOffer != nil,
                   await sub.isEligibleForIntroOffer {
                    eligible.insert(product.id)
                }
            }
            trialEligible = eligible
        } catch {
            loadFailed = true
            message = "Could not load prices: \(error.localizedDescription)"
        }
    }

    private func rank(_ id: String) -> Int { ProPlan.allIds.firstIndex(of: id) ?? Int.max }

    public func purchase(_ product: Product) async {
        isBusy = true
        defer { isBusy = false }
        do {
            switch try await product.purchase() {
            case .success(let result):
                switch result {
                case .verified(let transaction):
                    await transaction.finish()
                    await refreshEntitlements()
                case .unverified:
                    message = "The purchase could not be verified."
                }
            case .pending:
                message = "The purchase is waiting for approval."
            case .userCancelled:
                break
            @unknown default:
                break
            }
        } catch {
            message = "Purchase failed: \(error.localizedDescription)"
        }
    }

    /// Asks the App Store for the Apple ID's purchases again (the "Restore Purchases" button).
    public func restore() async {
        isBusy = true
        defer { isBusy = false }
        do {
            try await AppStore.sync()
            await refreshEntitlements()
            message = isPro ? "Purchases restored." : "No purchases found for this Apple ID."
        } catch {
            message = "Restore failed: \(error.localizedDescription)"
        }
    }

    public func refreshEntitlements() async {
        var best: ProPlan?
        for await entitlement in Transaction.currentEntitlements {
            guard case .verified(let t) = entitlement else { continue }
            let snapshot = EntitlementSnapshot(productId: t.productID, expirationDate: t.expirationDate, revocationDate: t.revocationDate)
            guard snapshot.grantsPro(), let plan = ProPlan(rawValue: t.productID) else { continue }
            // Lifetime wins over any subscription.
            if best == nil || plan == .lifetime { best = plan }
        }
        activePlan = best
        isPro = best != nil
    }
}

extension Product {
    /// "3 days free" for a free-trial introductory offer, else nil.
    var trialDescription: String? {
        guard let offer = subscription?.introductoryOffer, offer.paymentMode == .freeTrial else { return nil }
        let n = offer.period.value
        let unit: String
        switch offer.period.unit {
        case .day: unit = n == 1 ? "day" : "days"
        case .week: unit = n == 1 ? "week" : "weeks"
        case .month: unit = n == 1 ? "month" : "months"
        case .year: unit = n == 1 ? "year" : "years"
        @unknown default: unit = "days"
        }
        return "\(n) \(unit) free"
    }
}
