# 22. Apple Watch companion: approvals on the wrist

- **Slug**: `watch-approvals`
- **Milestone**: M23
- **Status**: stage 1 implemented (relay without push); stage 2 (push wake-up) waits for the owner's decision on the channel. Not yet run on a watch (no simulator pairing here); builds for watchOS device and simulator.
- **Roles**: `doc-dev` → `rev` → `swift-bridge-dev` → `ios-ui-dev` → `rev` → `hacker` (Rust core unchanged for stage 1)
- **Hacker gate**: **Yes** (approval decisions, signing key, WatchConnectivity channel, notification actions)
- **Builds on**: `13-host-approval-broker.md` (queue, SSHSIG decisions), `19-hooks-templates-and-host-guard.md` (analysis flags)

## 1. Problem and scope
An agent waits for an approval (120 s window) while the phone is in a pocket. The user wants to see the request and
approve or deny from an Apple Watch. In scope: a watchOS app (Approve / Deny / details, notices), actionable notifications,
Watch ↔ iPhone relay. Out of scope: SSH from the watch, a key on the watch, terminals on the watch, complications beyond a
pending-count (later).

## 2. Design decisions
1. **The iPhone signs, the watch only relays the verdict.** The Ed25519 key stays in the iPhone Keychain; the host trusts only
   that key (`allowed_signers`). A watch-held key would need a second trusted key on every host and a Keychain/Rust-core
   build for watchOS. The watch also cannot reach the tailnet by itself (Tailscale is not on the watch), so a direct SSH
   connection is not an option.
2. **The watch sends `{id, verdict}` only.** The phone then re-reads the pending request from the host
   (`list_pending_approvals`), checks that `id` is still pending and not expired, shows the same fields it would show on its
   own screen, and signs exactly what the host holds (`command_sha256` binds what the user saw). A forged or replayed watch
   message can at most pick a verdict for a request that really is pending; it cannot change the command or invent a request.
3. **Never auto-approve.** Unreachable phone, expired request, mismatching content, host error → the watch shows the state
   and the agent falls back to its native prompt after the timeout, exactly like today.
4. **Delivery is the hard part.** Requests are born on the host; the phone app only polls in the foreground (Zero Battery
   Drain: the socket is closed in the background). Nothing wakes the phone or the watch today (Feature 13 §9 keeps push as
   backlog). Stage 1 therefore works when the phone app is in the foreground (watch mirrors it live). Real wrist approvals
   need a wake-up channel:
   - **A. APNs relay** (hook → small relay → APNs → phone → watch). Needs a server we operate, device tokens and an Apple
     push certificate/key; the payload must carry no command text (only "approval pending", host name).
   - **B. ntfy-style topic** (self-hosted or public ntfy) used as the push source: no own server, but another trusted party
     and an app-side subscription.
   - **C. Time-boxed background poll** while an agent session is active (`BGAppRefresh` is too coarse; not viable for a
     120 s window).
   Decision pending with the owner; stage 1 ships without it and documents the limit. Recommended: A, payload-minimal.

## 3. Components
```
 host hook ──(push, stage 2)──► iPhone (UNNotification w/ actions) ──mirrors──► Watch notification  [Approve][Deny]
 iPhone app ── WatchConnectivity (applicationContext = pending list; sendMessage = verdict) ── Watch app
```
- **iOS side**: `WatchBridge` (WCSession delegate): publishes a *sanitised* pending list to the watch (id, agent, tool, command
  tail ≤ 160 chars, cwd basename, analysis flags, expires, host name) via `updateApplicationContext`; receives
  `{id, verdict}` via `sendMessage`/`transferUserInfo`; hands it to `ApprovalsCenter.decide(id, verdict, source: .watch)`, which
  runs the normal path (re-read from the host, sign, submit). Notification actions (`UNNotificationAction` Approve/Deny,
  not `.foreground`, `authenticationRequired`) go through the same `decide`.
- **Watch app** (SwiftUI, watchOS 10): list of pending requests with countdown, detail with full command tail and flags
  (e.g. "rm -r", "network", "sudo"), Approve / Deny buttons (Deny first, confirmation on Approve for requests flagged risky),
  notice list, state line "iPhone not reachable". Haptic `.notification` on arrival and `.failure` on expiry.
- **No secrets on the watch**: no key, no host address, no server credentials. The list holds only what the approval card
  already shows and is cleared when expired.

## 4. Protocol (WatchConnectivity)
```json
phone → watch (applicationContext): {"v":1,"seq":42,"host_time":1760640003,"items":[{"id":"<32 hex>","agent":"claude","tool":"Bash","tail":"git push origin main","dir":"proj","flags":["network"],"expires":1760640120,"server":"MacBook"}]}
watch → phone (sendMessage):        {"v":1,"id":"<32 hex>","verdict":"allow|deny","seq":42}
phone → watch (reply):              {"ok":true,"state":"signed|expired|not_pending|host_unreachable|rejected"}
```
Validation on the phone: `id` matches `^[0-9a-f]{32}$`, `verdict` in the set, `seq` is not older than the last published, unknown
keys ignored; one verdict per id (second one answers `not_pending`). Messages from anything but the paired watch cannot occur
(WCSession is per paired device); the phone still treats the content as untrusted input.

## 5. States and lifecycle
- Watch: `NoPhone` (not reachable) · `Idle` · `Pending(n)` · `Sending` · `Result(state)`.
- Phone side: the pending list is published after every foreground poll and cleared on background/disconnect; a stale list on
  the watch is dropped by `expires` and by `host_time` drift check.
- Battery: no polling on the watch; it renders what the phone pushes. Phone behaviour in the background is unchanged.

## 6. Security considerations (for `hacker`)
- A compromised/jailbroken watch or a replayed message can only choose a verdict for a **currently pending** request; the
  phone re-fetches and signs the host's own copy. A "bulk approve" / "always allow" action does not exist on the watch.
- Wrist-raise accidents: Approve needs a deliberate tap, risky requests need a confirm step; Deny is the default/first button.
- Notification actions: `authenticationRequired` so a locked phone does not approve from the lock screen; push payloads (stage
  2) contain no command text, so a lock-screen preview leaks nothing.
- Watch UI shows host-supplied text: strip control/bidi characters (`display_safe` logic mirrored in Swift), cap length.
- Push channel (stage 2): payload authenticity — the relay must not be able to forge an approval; it can only say "look at the
  host"; the phone always asks the host over SSH. Device token handling and the relay's retention policy get their own audit.
- Logging: no command text in logs on either device.

## 7. Verification
- Swift unit tests: message validation (bad id/verdict/seq/unknown keys), `ApprovalsCenter.decide` source `.watch` goes through
  re-read + sign, expired/non-pending/second verdict paths, list sanitising (length, bidi), no double submit.
- Rust: none new in stage 1 (existing 15 hook-script + loopback approval round trip cover signing/verification).
- Manual on device (no simulator on the dev machine): paired watch, foreground approve/deny, phone unreachable, expiry, risky
  confirm, notification action with locked phone.

## 8. Stages
1. **Stage 1** (no push): WatchBridge, watch app, WatchConnectivity relay, notification categories; works while the phone app is
   in the foreground or reachable. Documents the delivery limit.
2. **Stage 2**: push wake-up (A recommended) incl. relay spec, token registration, hacker audit.
3. **Stage 3**: complication with the pending count, quick "deny all expired" cleanup, Smart Stack widget.

## 9. Open items
- Owner decision on the push channel (A/B).
- watchOS target (`SSHAppWatch`, bundle `com.semenov.sshapp.watchkitapp`) in the hand-edited `project.pbxproj`, signing for the
  watch app, embedding in the iOS app.
- Whether risky-flag logic is shared from the Rust `analyze_command` result already attached to requests (preferred: reuse).
- Payments (off): approvals on the watch stay free.

## 10. Implementation notes (stage 1)
- **Shared**: `ios/Sources/Shared/WatchProtocol.swift` (compiled into the iOS app, the watch app and the SPM test target): `WatchApproval`,
  `WatchContext`, strict id check (32 lowercase hex), `sanitize` (control and bidi characters removed, length caps, `keepEnd` for the
  command tail), `parseVerdict` (version, id, `allow`/`deny`, `seq`; unknown keys ignored), `WatchReply`.
- **iPhone**: `Services/WatchBridge.swift` publishes up to 5 approvals (notices are not sent) with the command tail (160 characters),
  last folder name, analysis flags and seconds left on the host's clock via `updateApplicationContext`; it publishes an empty list when
  the foreground poll stops (`ApprovalsCenter.isPolling`), so the watch never shows stale requests. A verdict goes
  `parse → seq not from the future → exactly one matching pending approval → not expired → ApprovalsCenter.decide` (re-reads the host,
  signs the host's copy). Replies: `signed`, `expired`, `not_pending`, `host_unreachable`, `rejected`.
- **Watch** `ios/SSHAppWatch` (watchOS 10, companion `com.semenov.sshapp.watchkitapp`, no key, no host address): list with countdown,
  detail with the full tail and flags, **Deny first**, Approve asks for confirmation when the request has flags, haptics on arrival and
  result, "iPhone not reachable" state. Nothing is approved automatically; a failed send leaves the request for the agent's own prompt.
- **Not in stage 1**: notification actions and any wake-up of a suspended phone app (needs the push channel, section 2.4), complication.
- **Tests**: `WatchProtocolTests` (ids, verdict validation incl. forged/odd messages, context limits, text cleaning, list building
  from approvals only); the protocol file was also executed on macOS with a standalone script. Swift tests are built, not run (no
  simulator).
