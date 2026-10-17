# Security review: Apple Watch approvals (stage 1), 2026-10-17

Scope: `WatchProtocol`, `WatchBridge`, `ios/SSHAppWatch`, the `ApprovalsCenter.decide` path they use.

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | Info | A message from the watch can only choose allow/deny for an id that is currently pending on the phone; the command, the key and the host copy never come from the watch (the core signs what the host holds) | By design, covered by `parseVerdict` and `handle(verdict:)` checks |
| 2 | Info | Replay: a second verdict for the same id finds no pending entry (`not_pending`); a verdict with `seq` newer than anything published is rejected | Verified in code, unit-tested for parsing; the replay path needs a device test |
| 3 | Low | Host-supplied text (command, tool, folder) is shown on the wrist | Fixed in design: control and bidirectional characters removed, caps, end of long commands kept |
| 4 | Low | Wrist accident: Approve on a flagged request | Mitigated: Deny first, confirmation for flagged requests; no "always allow" exists on the watch |
| 5 | Info | Stale list on the watch while the phone app is suspended | Mitigated: empty list is published when the foreground poll stops; the watch drops entries by their countdown |
| 6 | Info | Locked phone | Not applicable in stage 1 (the phone app must be in the foreground, i.e. unlocked); notification actions of stage 2 must use `authenticationRequired` |
| 7 | Info | No secrets on the watch (no key, host address or credentials; only the sanitised approval fields) | Verified by reading `SSHAppWatch` and the context type |

Open for stage 2 (push): payload without command text, relay cannot forge an approval (the phone always asks the host over SSH), token
handling and retention.
