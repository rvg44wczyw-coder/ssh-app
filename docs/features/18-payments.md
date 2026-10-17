# 18. Payments: Pro subscription and lifetime purchase (StoreKit 2)

## Plans
Products (ids in `ProPlan`, must match App Store Connect): `com.semenov.sshapp.pro.weekly`, `.monthly`, `.yearly` (auto-renewable, one subscription group "SSH-App Pro") and `.lifetime` (non-consumable). Free trial: 3 days on weekly, 7 days on monthly and yearly (introductory offer, configured in App Store Connect; the app only shows it when the Apple ID is eligible). **Prices are not final**: `ios/SSHApp.storekit` carries placeholder test prices; real ones are set in App Store Connect later.

## Free tier and Pro
- Free: up to 20 open sessions (tabs) and 3 servers. Pro (any active plan) removes both limits.
- Only adding is refused; tabs and servers above the limit (e.g. after a subscription ends) are kept and keep working.
- Gate points: new tab (`+`, home card, agent preset, agent profile launch, Settings → Agents launch, opening a tmux session that is not already a tab) in `MainTerminalView.canOpenNewSession()`; "Add Server" in Settings → Servers. Past a limit the paywall opens.

## Implementation
- `Services/SubscriptionManager.swift`: `ProPlan`, `FreeTier`, `Monetization`, `EntitlementSnapshot` (pure rule for what a transaction grants), `SubscriptionManager` (load products, purchase, restore, `Transaction.updates` listener, `Transaction.currentEntitlements`). Verification is on the device only (signed JWS transactions, `.verified`); no server, no receipts sent anywhere. Offline, the last known entitlements of StoreKit are used.
- `Views/PaywallView.swift`: benefits, plans with trial text and price, **Restore Purchases** (`AppStore.sync()`), Manage subscription, auto-renewal terms. Opened from a limit or from Settings → SSH-App Pro.

## On/off switch
The whole payment system is optional. It is a build option: the compilation condition `PAYMENTS` (Active Compilation Conditions). **On `main` it is currently off** (the app is free for now: no limits, no paywall, no Settings row); `feature/payments` has it on for Debug and Release. To switch on: add `PAYMENTS` to the project's Active Compilation Conditions (Debug: `DEBUG PAYMENTS`, Release: `PAYMENTS`). Remove it and the app has no limits, no paywall and no Settings row. Debug builds also have a runtime switch in Settings → SSH-App Pro. There is deliberately no switch in release builds, otherwise users could turn the limits off.

## Testing locally
Select `ios/SSHApp.storekit` under Product → Scheme → Edit Scheme → Run → Options → StoreKit Configuration (the repo has no shared scheme). Then purchases, trials, renewals and refunds are simulated in Xcode (Debug → StoreKit → Manage Transactions). The unit tests (`SubscriptionTests`) cover limits, the on/off rule and entitlement expiry/revocation; they build but have not been run (no simulator on this machine).

## Branch and worktree
The work lives on branch `feature/payments` in its own git worktree, `../ssh-app-payments` (next to the main checkout), so it can be built, changed and reviewed without touching `main`.
- Work there: `cd ../ssh-app-payments` (build with `xcodebuild -project ios/SSHApp.xcodeproj -scheme SSHApp ...` or `make iphone`; the worktree has its own `.build`).
- Bring `main` changes in: `git merge main` (or rebase) inside the worktree.
- Ship: after `rev` and `hacker`, merge `feature/payments` into `main`, then `git worktree remove ../ssh-app-payments` and `git branch -d feature/payments`.
- The first commit of the feature (`583cc2b`) was made on `main` before the branch existed; `main` still contains it until it is reset or reverted there. Until that is done the two branches share the commit and the feature is not isolated.

## Open
- Real prices, localized names and the subscription group in App Store Connect; screenshot of the paywall for review.
- Privacy Policy and Terms of Use links are required by App Review next to the subscription terms; the URLs do not exist yet.
- Family Sharing, promo/offer codes, and "billing retry" messaging are not handled.
- `rev`/`hacker` not yet run on this feature. Client-side-only entitlement can be bypassed on a jailbroken device; accepted for the first version (no server).
