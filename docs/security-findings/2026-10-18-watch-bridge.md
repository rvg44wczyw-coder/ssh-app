# Security review: Apple Watch approval relay (Feature 22, stage 1)

Date: 2026-10-18. Code analysis only: WatchConnectivity cannot run without a paired watch, so no
live attack was possible; the protocol functions are covered by `WatchProtocolTests` (compiled, not
run) and were executed standalone on macOS earlier.

## 1. Scope
`ios/Sources/Shared/WatchProtocol.swift`, `ios/Sources/SSHApp/Services/WatchBridge.swift`
(`handle(verdict:)`, `items(from:)`, publishing), watch app UI contract (spec section 10).
Applied: input validation of watch messages, authorisation (who may approve what), replay,
information leakage to the watch. Ruled out: MITM (WatchConnectivity is the system's paired-device
channel; the app adds no network path), crypto (the watch holds no key; the signature is made on
the phone over the host's own copy), downgrade (single protocol version, unknown versions dropped).

## 2. Findings
None above Info.

### Checked and held
- **Verdict validation**: `parseVerdict` requires version, a 32-hex id, `allow`/`deny` and an
  integer `seq`; anything else is dropped. A verdict for an id that is not exactly one currently
  pending *approval* (notices excluded) gets `not_pending`; an expired one `expired`.
- **No approval from a stale or foreign request**: the phone signs `center.decide(entry)`, which
  re-reads the request from the host; the watch cannot supply a command or a digest.
- **Long commands**: when the command did not fit the watch preview, the phone itself refuses an
  `allow` (`rejected`), not only the watch UI. Deny always works.
- **Replay**: `seq` may not be from the future. An old verdict can only act on an id that is still
  pending and unexpired; a replayed allow cannot resurrect a denied/expired request (the entry is gone).
- **Stale list on the watch**: an empty list is published when the foreground poll stops, so the
  watch does not keep offering requests the phone can no longer answer; a verdict then gets
  `not_pending`.
- **Flood**: each message does a dictionary parse and a lookup in at most the pending entries; only a
  matching pending id reaches the host.

### Info
1. Command text (tail, up to 160 characters) is mirrored to the watch with control and bidi
   characters removed; a command containing a secret shows that secret on the watch, as it already
   does on the phone's approval card. The context is the latest application context only.
2. `seq <= self.seq` is deliberately lenient: a verdict sent just after the phone republished must
   still be honoured. The id/pending/expiry checks make staleness harmless.
3. A flagged (destructive-looking) command is approved from the watch after the watch's own
   confirmation; the phone does not add a second one. Consistent with the phone card, where the
   flags are advisory.

## 3. Verdict
Clean (Info only).
