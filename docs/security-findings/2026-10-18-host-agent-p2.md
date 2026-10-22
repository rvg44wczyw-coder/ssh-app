# Security pass: host agent and rpc (P2, feature 26), 2026-10-18

## 1. Scope

`crates/host/src/{agent,rpc}.rs`, tests `crates/host/tests/agent.rs`. Applied: local IPC parsing,
resource exhaustion, privilege/identity of the socket, process lifetime. Ruled out: crypto (none
here), network (no TCP). Live (own temp home under /tmp, release binary): agent start via rpc,
reuse by a second rpc, 4 KiB line cap, unknown/`exec` method, repeated `subscribe`, lease expiry
(exit and socket removal), SIGKILL then recovery of the stale socket, idle cost (0.0 % CPU, 2.6 MB
RSS, state `S`). Cross-user peer rejection is code analysis only (one user on this machine).

## 2. Findings

1. **Unbounded subscriptions** [DoS, Low] — repeated `subscribe` on one connection added a sender
   and a thread each time. **Fixed**: one per connection, at most 16 in total; test
   `one_subscription_per_connection`.
2. **Same-user process can extend the lease or fake state** [SandboxEscape, Low, accepted] — any
   process of the user (including the model) can connect, renew the lease (capped at 24 h per call;
   no cap on repeated calls) or write hook files that change derived state. State is advisory, the
   agent has no mutating or exec method, approvals still need the phone's signature. The cost is a
   longer-lived idle process; stated in the spec.
3. **Slow subscriber** [DoS, Low, accepted] — a subscriber that never reads blocks its own writer
   thread; queued events are small and only produced on state changes (bounded by sessions).

No finding: socket in a 0700 directory, mode 0600, `getpeereid` uid check; path length cap;
single instance by `flock`; no shell, no file writes beyond the socket and lock; request size cap.

## 3. Verdict

Findings (highest severity Low); the fixable one is fixed.

## Update 2026-10-22

Finding 3 (slow subscriber) is fixed: every connection has a 10 s write timeout (`bounded_writer`), so a subscriber that never reads loses its connection instead of holding its writer thread. Finding 2 stays accepted: a lease that a same-user process renews is the lease working as designed, and the agent has no mutating method.
