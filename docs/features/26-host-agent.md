# Feature 26: `ssh-app-host agent` / `rpc` — the host agent that lives with the phone (phase P2)

Status: spec (2026-10-18). Follows 24 (helper) and 25 (hook). Owner decision (16 section 11):
resident process **only while a phone session exists**; macOS only; push is P3 (blocked on the
Apple key / Cloudflare account) and plugs into the event sink defined here.

## 1. Goal

Give the phone live agent state without re-scanning on every request, and give P3 one place that
notices "needs approval / finished / waiting" the moment it happens. No always-on process.

## 2. Design

### 2.1 Lifetime = a lease, not a launchd job

There is **no launchd plist**. The app opens `ssh-app-host rpc --lease SECS` over SSH; `rpc` starts
the agent if it is not running (detached, `setsid`, stdio to /dev/null) and bridges stdio to it.
Every connection renews the agent's lease (default 12 h, at most 24 h, never shortened). When the
lease expires the agent removes its socket and exits, even if clients are connected. So: no phone
contact for the lease length, no process; idle CPU is 0 % (blocked in `kqueue`/`kevent` with a
timeout equal to the lease remainder). Because the agent is started only by a connecting phone
there is no restart loop and no crash-loop guard is needed; a crash costs one missing agent until
the next connect.

### 2.2 Socket and single instance

`~/.ssh-app/run/hostd.sock`, directory 0700, socket 0600, `flock` on `run/hostd.lock` for single
instance (the loser exits), stale socket removed under the lock. Peer uid is checked with
`getpeereid` and must equal ours. Path longer than 100 bytes is refused.
Max 16 connections; request lines at most 4 KiB; unknown methods get an error.

### 2.3 Protocol (newline-delimited JSON, closed method list, no exec)

| Request | Reply |
|---|---|
| `{"id":1,"m":"hello"}` | `{"id":1,"ok":true,"name":"ssh-app-host-agent","version":..,"pid":..,"lease_until":..}` |
| `{"id":2,"m":"state"}` | `{"id":2,"ok":true,"host_time":..,"sessions":[{agent,session,tmux,cwd,state,ts}],"pending":N}` |
| `{"id":3,"m":"subscribe"}` | `{"id":3,"ok":true}` then `{"event":"state","agent":..,"session":..,"from":..,"to":..}` per change |
| `{"id":4,"m":"lease","secs":N}` | extends the lease (capped), replies with `lease_until` |

State per (agent, session), derived only from files the hooks already write: `approval` (an
approval request is pending), `finished` (newest notify is `Stop`), `waiting` (newest notify is
`Notification` or another event), `running` (registered by SessionStart, nothing newer).
The agent never decides anything, runs nothing, and has no mutating method (approvals are still
answered by `ctl decide`, signed, as today).

### 2.4 Events and the P3 hook point

The watcher re-derives the state map when `run/pending` or `run/sessions` change (kqueue vnode
events) and publishes diffs to subscribers and to an `EventSink`. P2 ships the sink trait with a
no-op implementation; P3 adds the APNs sink, which maps `to` ∈ {approval, finished, waiting} to the
push category and never includes text.

### 2.5 `rpc`

`ssh-app-host rpc [--lease SECS]`: ensure agent (connect; on failure spawn `current_exe agent` and
retry for up to 3 s), send `lease`, then copy stdin to the socket and the socket to stdout; exits
when either side closes (SSH drop = stdin EOF). The app's backgrounding closes the TCP socket as
today, the agent keeps running until its lease ends.

## 3. Security

Same-user trust: the model can write the files the state is derived from, so state is advisory (it
can fake "finished", which is a notification, not an approval). Socket is private and uid-checked;
the method list is closed; inputs are capped; the binary is the pinned helper; no TCP port.

## 4. Tests

Host crate integration tests with a private `SSH_APP_HOME` under `/tmp` (short socket path) and no
tmux: agent starts via `rpc`, second `rpc` reuses it (same pid), `state` follows `hook` calls
(`Stop` → `finished`, pending approval → `approval`), `subscribe` receives the transition, lease of
2 s makes the agent exit and remove the socket, socket mode 0600, oversized line and unknown method
rejected, stale socket after a killed agent is recovered.

## 5. Not in P2

Core/Swift wiring (a typed `host_agent_state` call and a status row), `tmux -C`, the incremental
usage index, push. They follow once the agent is proven by the tests above.

## 6. Implementation notes (2026-10-18)

Done: `agent` (lease, socket, `getpeereid`, kqueue watcher on `run/pending` and `run/sessions`,
state derivation and diff, `EventSink`), `rpc`, subcommands `agent` and `rpc`, `hello` cap
`agent`. Tests `crates/host/tests/agent.rs` (reuse, state and events, closed method list, lease
exit, stale socket, one subscription per connection). Live: idle 0.0 % CPU, 2.6 MB RSS. Security
pass: `docs/security-findings/2026-10-18-host-agent-p2.md`. Core/Swift wiring done: `host_agent_state(expected_sha256)` (pinned helper only, starts the agent
through `rpc`, lease 12 h), Agent row in the Host service card (user-triggered, never automatic),
loopback test starts and stops a real agent. Not done: `tmux -C`, usage index.
