import SwiftUI
import StoreKit

/// Plans, trial conditions, purchase and Restore Purchases.
struct PaywallView: View {
    /// Why the screen opened (a limit was reached), or nil when opened from Settings.
    var reason: String? = nil
    /// Pushed inside Settings (no own navigation stack or Done button).
    var embedded = false
    @ObservedObject private var subs = SubscriptionManager.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        if embedded { content } else { NavigationStack { content } }
    }

    private var content: some View {
            List {
                #if DEBUG
                Section {
                    Toggle("Payment system (debug)", isOn: Binding(get: { subs.paymentsEnabled }, set: { subs.setPaymentsEnabled($0) }))
                } footer: {
                    Text("Debug builds only. Release builds follow the PAYMENTS build option; without it there are no limits and no paywall.")
                }
                #endif
                if let reason {
                    Section { Text(reason).font(.subheadline) }
                }
                Section {
                    benefit("infinity", "Unlimited sessions", "Free: \(FreeTier.maxSessions) open tabs")
                    benefit("server.rack", "Unlimited servers", "Free: \(FreeTier.maxServers) servers")
                } header: {
                    Text("Pro")
                }

                if !subs.paymentsEnabled {
                    Section { Text("The payment system is off: everything is unlocked.").foregroundColor(.secondary) }
                } else if subs.isPro {
                    Section {
                        Label("Pro is active" + (subs.activePlan.map { " (\($0.title))" } ?? ""), systemImage: "checkmark.seal.fill")
                            .foregroundColor(.green)
                        if subs.activePlan?.isSubscription == true {
                            Button("Manage subscription") { openManage() }
                        }
                    }
                } else {
                    Section {
                        if subs.products.isEmpty {
                            if subs.loadFailed {
                                Button("Reload prices") { Task { await subs.loadProducts() } }
                            } else {
                                HStack { ProgressView(); Text("Loading prices…").foregroundColor(.secondary) }
                            }
                        }
                        ForEach(subs.products, id: \.id) { product in
                            planRow(product)
                        }
                    } footer: {
                        Text(footer)
                    }
                }

                Section {
                    Button {
                        Task { await subs.restore() }
                    } label: {
                        HStack {
                            Label("Restore Purchases", systemImage: "arrow.clockwise")
                            if subs.isBusy { Spacer(); ProgressView() }
                        }
                    }
                    .disabled(subs.isBusy)
                }
            }
            .navigationTitle("SSH-App Pro")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if !embedded { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            }
            .alert("", isPresented: Binding(get: { subs.message != nil }, set: { if !$0 { subs.message = nil } })) {
                Button("OK", role: .cancel) {}
            } message: { Text(subs.message ?? "") }
    }

    private var footer: String {
        "Subscriptions renew automatically until cancelled at least 24 hours before the period ends. Payment is charged to your Apple ID; manage or cancel in Settings → Apple ID → Subscriptions. A free trial turns into a paid subscription unless cancelled before it ends. The lifetime plan is a single payment."
    }

    private func benefit(_ symbol: String, _ title: String, _ detail: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: symbol).foregroundColor(.appAccent).frame(width: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail).font(.caption).foregroundColor(.secondary)
            }
        }
    }

    private func planRow(_ product: Product) -> some View {
        let plan = ProPlan(rawValue: product.id)
        let trial = subs.trialEligible.contains(product.id) ? product.trialDescription : nil
        return Button {
            Task { await subs.purchase(product) }
        } label: {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(plan?.title ?? product.displayName).font(.body.weight(.semibold))
                    if let trial {
                        Text("\(trial), then \(product.displayPrice) \(plan?.periodNote ?? "")")
                            .font(.caption).foregroundColor(.secondary)
                    } else {
                        Text(plan?.periodNote ?? "").font(.caption).foregroundColor(.secondary)
                    }
                }
                Spacer()
                Text(product.displayPrice).font(.body.weight(.semibold))
            }
        }
        .disabled(subs.isBusy)
    }

    private func openManage() {
        #if canImport(UIKit)
        if let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene {
            Task { try? await AppStore.showManageSubscriptions(in: scene) }
        }
        #endif
    }
}
