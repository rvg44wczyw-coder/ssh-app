# Feature 16: Host-side service — what it is for, and how to build it

> **Note (2026-10-26):** parts of this document that describe the Unix-socket approval scheme are superseded by the file queue (`~/.ssh-app/run/{pending,decisions}`, signed by the phone key). See specs 24, 57 and 62.

**Status:** analysis (2026-10-17). No code yet. Builds on features 03, 09, 11, 13, 15 and the
usage probe (`crates/core/src/usage.rs`). Decisions that need the owner are in §9.

## 1. Where the current design hurts

Today the host runs **nothing** of ours between requests. Everything is either an SSH exec started by
the phone (telemetry, usage, file browser, transcript) or a hook the agent starts (the approval
broker of feature 13). That was a deliberate choice (feature 03 §1: no resident daemon, no listening
port, nothing proprietary in the path). It costs us:

| Pain | Why it exists today |
|---|---|
| Approvals and "agent finished" reach the phone only while the app is in the foreground (feature 13 §9) | Nothing on the host can reach Apple's push service; after 120 s the agent falls back to its own prompt |
| The sessions menu and limits re-scan transcripts on every open (`usage_probe.py`, up to 15 s) | No incremental index; no state survives between requests |
| ~600 lines of Python/shell embedded in Rust strings (usage, telemetry, hook, tmux attach) | Host logic has no typed protocol, no versioning, no unit tests of its own; parsing is by `===MARKER===` text |
| "Which agent runs in which tmux session" is guessed from the process tree | No registration; SessionStart hooks are not installed |
| Approvals are polled every 2 s while open | No streaming channel |
| The widget can only do one-shot SSH snapshots (`fetch_host_telemetry_once`) | No history, no cheap summary, extension cannot hold a connection |
| Detecting "agent is waiting for input" without a hook (Codex/Grok unverified, plain `ollama run`, a shell) is impossible | Nothing watches the pane |
| Host-side hard rules ("never touch `~/.ssh`") exist only as agent permission files the model can edit (feature 15 §3.2) | No independent enforcement point |

Most of these do **not** need a long-running process. §3 separates what needs one from what only
needs a better host helper.

## 2. What is worth building, ranked

"Resident" = needs a process alive between phone connections. "Egress" = needs the host to make
outbound network calls (new trust surface).

| # | Capability | Value | Resident | Egress | Notes |
|---|---|---|---|---|---|
| 1 | **Push wake-up** for approval needed / agent finished / agent waiting | Highest: the one thing the app cannot do with the app closed | No for approvals (the hook can POST to the Worker itself); **yes** for "finished/waiting" detection outside hooks | Yes (Worker only) | `scripts/apns-worker/worker.ts` already exists (zero-knowledge, opaque token, empty ping) |
| 2 | **Typed host helper** `ssh-app-host` (one Rust binary) replacing embedded Python/shell | High: testable, versioned, faster, removes `python3` dependency for usage/telemetry | No | No | Subcommands: `usage`, `telemetry`, `sessions`, `ctl` (today's broker), `hook` (today's script) |
| 3 | **Agent state machine** per tmux session: working / waiting-for-input / idle / finished, from hooks + pane tail | High: tab badges, "N need you", push triggers | Yes (or hooks write state and the helper only reads) | No | Hooks (Claude/agy `Stop`, `Notification`, `SessionStart`) cover the agents we know; pane heuristics are the fallback and are fragile |
| 4 | **Incremental usage index** (transcripts → SQLite, exact 5 h / 7 d windows, per profile) | Medium: instant sessions menu, trend, threshold alerts ("80 % of the window") | Yes for live alerts; no for on-demand index with a cache file | No | Still only token totals; real subscription quota has no safe source |
| 5 | **Streaming channel** (phone ↔ host over one SSH exec, JSON frames): approvals long-poll, live usage, streaming Ollama replies | Medium: replaces 2 s polling, token-by-token chat | No: `ssh-app-host rpc` lives only as long as the channel | No | Needs a streaming exec API in the core (today `collect_output` waits for EOF) |
| 6 | **Host rule engine** (`PreToolUse` deny rules evaluated on the host, no phone) | Medium: closes the "model edits its own permissions" hole offline | No (it runs inside the hook) | No | Config the model cannot silently change: owned by the helper, edited from the phone with a signature |
| 7 | **Telemetry history** (CPU/RAM/disk/battery series, ring buffer) for the monitor and widget | Low–medium | Yes | No | Only useful with a widget or alerts; skip until the widget exists |
| 8 | **Alerts**: disk < 5 %, battery < 10 % while on battery, a watched process died, a long command finished | Low–medium | Yes | Yes (push) | "Command finished" needs a shell hook (`precmd`), invasive |
| 9 | **Change summaries** after an agent turn (git diff stat, files touched) | Medium | No (computed on `Stop`) | No | Natural payload for the finished notification, but it must stay on the host and be fetched over SSH after the wake-up |
| 10 | Local-model proxy / RAG / project indexing | Speculative | Yes | No | Not recommended: scope creep, large attack surface |

**Reading of the table:** items 2, 5, 6, 9 are better tooling and need no daemon. Item 1 (approvals)
needs only an outbound POST from the hook. A resident process is justified by **3 and 4 (live
state, alerts) and by "agent finished/waiting" push** where no hook exists. That is a smaller
daemon than "everything on the host".

## 3. Architecture options

| Option | What runs | Pros | Cons | Verdict |
|---|---|---|---|---|
| A. Status quo | Nothing; SSH exec + hooks | Zero surface | Items above stay impossible; embedded scripts keep growing | Keep as the **fallback path** |
| B. On-demand helper | `ssh-app-host` binary, started per request or per channel | Typed JSON, versioned, tested; no resident process | Must be installed and updated on the host | **Do first** |
| C. User daemon behind the helper | B plus a per-user launchd agent (systemd `--user` on Linux) that owns a **Unix socket** `~/.ssh-app/run/hostd.sock` (0600); the helper's `rpc` subcommand bridges stdio to it | Live state, alerts, push; **no TCP port**, auth is SSH, host key pinned as today | A resident process: battery, crashes, updates | **Do second**, event-driven only |
| D. Network daemon (HTTP/gRPC on the tailnet) | Listens on 100.x | Widget can call it directly | New auth + TLS, listening socket, exactly what feature 03 rejected | **Reject** |

### Recommended shape (B then C)

```
 iPhone app ──SSH (Tailscale, pinned host key, Ed25519)──► sshd
                                                             │ exec: ssh-app-host rpc      (stdio JSON frames)
                                                             ▼
                                              ssh-app-host ──unix socket 0600──► hostd (launchd agent)
                                                   ▲                                  │ kqueue/FSEvents: ~/.ssh-app/run,
   agent hooks: ssh-app-host hook <agent> <event> ─┘                                  │ transcripts, tmux (-C control mode)
                                                                                      ▼
                                                                SQLite (~/.ssh-app/state, WAL, 0600)
                                                                                      │ HTTPS POST, signed, opaque
                                                                                      ▼
                                                              apns-worker (Cloudflare) ──► APNs ──► phone wake-up
```

- **One binary, three roles**: CLI (`usage`, `telemetry`, `sessions`), hook entry (`hook <agent> <event>`,
  replacing `ssh-app-hook.py`), and the daemon (`hostd`). A `proto` crate holds the request/response
  types shared by the app core (UniFFI turns them into Swift records, so the app stops parsing text).
- **The hook never depends on the daemon.** It writes the request to the file queue exactly as
  today (`~/.ssh-app/run/pending`), pokes the daemon socket if it is there, and falls back to "no
  opinion" on every failure. A dead daemon degrades push, never approvals or agent work.
- **tmux**: use `tmux -C` (control mode) for pane output / session events instead of polling
  `list-panes`; one control client per server, only while some phone-visible agent session exists.
- **Event-driven**: kqueue/FSEvents on the queue and transcript directories, `tmux -C` stream,
  no timers while idle. Target: 0 % CPU idle, < 30 MB RSS, wake-ups only on events. A MacBook host
  matters here as much as the phone.

### Push path (zero-knowledge kept)

1. Phone registers once over SSH: `ssh-app-host push-register <apns-token> <worker-url>`; the host
   stores the token and a per-device Ed25519 public key (the app's existing key).
2. Event → daemon (or hook) → `POST worker {device_token, category, nonce, ts}` signed by a host-side
   push key. **No command, path, project, agent or message text leaves the machine**; `category` is one
   of `approval | finished | waiting | alert` and nothing else (feature 03 §4.1 rule stays).
3. The Worker forwards an APNs background/alert push with only a generic title. The app's
   Notification Service Extension (or the foreground launch) connects over SSH, fetches the detail
   from the host, and shows the real text locally. Notification content is therefore never in APNs.
4. Worker abuse protection: per-token rate limit, signature check, no logs of payloads.

## 4. Security analysis (the part that decides the design)

The agent and its model run **as the same user** as the daemon. Anything the daemon trusts from
the file system, the model can write. So:

| Threat | Mitigation |
|---|---|
| Model forges approvals | Unchanged: decisions need the phone's Ed25519 signature, verified on the host (feature 13). The daemon never decides anything. |
| Model edits daemon state / config to silence alerts or spam pushes | State is advisory only; push categories are fixed; rate limit per category; the phone shows "N pushes suppressed" if limited |
| Model uses the daemon as an execution primitive | **No exec method exists.** The RPC surface is a closed list of typed, read-mostly calls (list sessions, usage, telemetry, pending approvals, subscribe). Mutations are limited to: approval decision (signed), approval-key install, hook install (same guarded writer as today), push registration. No shell strings anywhere |
| Replacing the binary | The app ships the exact binary (built from this repo, embedded in the app bundle for macOS aarch64/x86_64, Linux later), writes it over SSH with a SHA-256 the app already knows, and refuses to run a host binary whose hash differs from the pinned one. No download from the internet on the host |
| Socket access by another local user | Folder 0700, socket 0600, peer credential check (`LOCAL_PEERCRED` / `SO_PEERCRED`) must equal the owner's uid |
| Push as a covert channel / metadata leak | Fixed categories, no content; Worker sees device token + timing only. Timing of "agent finished" is the residual leak and is stated in the docs |
| Daemon crash loop eats battery | launchd `ThrottleInterval`, exponential self-backoff, auto-disable after N crashes with a visible status in the app |
| Test code reaching the real host | Same rules as `AGENTS.md` ("Test isolation"): tests start the daemon with a temp `HOME`, a private tmux socket (`-S`, `TMUX` removed) and a private socket path; compare the real tmux pid before/after |

## 5. Protocol sketch

Framing: length-prefixed JSON (`u32` big-endian + body), versioned `{"v":1,"id":7,"method":…,"params":…}`.
`hello` negotiates the version and returns the helper's hash, build id and capabilities, so the app
can show "host service: running / older / absent" and fall back to option A when absent.

- Reads: `sessions.list`, `sessions.watch` (stream of state changes), `usage.report`, `telemetry.once`,
  `telemetry.history`, `approvals.pending`, `approvals.watch`.
- Writes (closed list): `approvals.decide` (signed), `hooks.install|remove`, `push.register|unregister`, `rules.set` (signed by the phone key).
- Everything returns typed errors; no free-form shell, no path arguments outside the existing validators.

## 6. Phased plan

| Phase | Deliverable | Needs a resident process | Exit criteria |
|---|---|---|---|
| P0 | `proto` + `ssh-app-host` CLI: `usage`, `telemetry`, `sessions`; streaming exec in the core; installer with pinned hash | No | Python usage probe and telemetry script deleted; loopback tests green; sessions menu opens < 1 s from a cache file |
| P1 | `ssh-app-host hook` replaces `ssh-app-hook.py`; `rpc` stdio channel; approvals long-poll | No | hook_script tests ported; no 2 s polling |
| P2 | `hostd` (launchd): agent state machine, SessionStart registration, incremental usage index, `tmux -C` | Yes | Idle CPU 0 %, RSS < 30 MB, survives tmux restart and sleep/wake, crash loop guard proven |
| P3 | Push: Worker hardening, device registration, NSE, categories, rate limits | Yes (outbound only) | App closed → approval/finished arrives on the phone; payload audit shows no content |
| P4 | Host rule engine + tamper detection + audit log | No | Rules survive the model editing agent settings; phone shows a warning on drift |
| P5 | Telemetry history, widget, alerts | Yes | Only after P2–P3 prove stable |

P0–P1 deliver most of the engineering value (typed, tested, fast) with **no** change to the threat
model. P2–P3 are the real decision (§9).

## 7. What the service must not do

No listening TCP port; no arbitrary command execution; no content in push payloads; no auto-update
from the network; no storage of secrets (API keys, tokens from agent configs); no reading of files
outside the agents' transcript/config directories; no behaviour change of agents on its own
(it observes and relays; the only enforcement is the opt-in rule hook).

## 8. Platform notes

- **Sleep**: a closed laptop runs nothing, so push cannot wake the phone for agents on a sleeping
  Mac. State this in the UI. `caffeinate -i` while an agent works is an optional setting
  (battery cost), not a default.
- **macOS**: launchd agent plist in `~/Library/LaunchAgents`, `KeepAlive` on failure only; FSEvents
  via the `notify` crate. **Linux**: systemd `--user` unit, inotify; the same binary.
- **Binary size/arch**: static Rust, aarch64 + x86_64 macOS first.
- The Python hook keeps working until P1 lands; both write the same queue format.

## 9. Decisions needed

1. **Is a resident process acceptable at all** (it reverses feature 03's "no daemon")? If no, stop
   at P0–P1 + hook-side push for approvals only; "finished/waiting" push then works only for agents
   with hooks.
2. **Push infrastructure**: deploy the Cloudflare Worker under your account (Apple developer key
   `.p8` as a secret) — yes/no. Without it P3 is impossible.
3. **Host targets**: macOS only (this laptop) or Linux servers too?
4. **Laptop battery**: accept an always-on event-driven agent (target 0 % idle), or start it only
   while a phone session exists?
5. **Distribution**: embed the host binary in the iOS app and install it over SSH (recommended)
   vs. a Homebrew formula.

## 10. Recommendation

Start with **P0** now (it is pure upside: the embedded Python goes away, usage and telemetry become
typed and testable, the sessions menu gets faster), then **P1**. Decide §9.1–§9.2 before P2; they
are the only questions with security weight. Each phase follows the dev-chain
(doc → rev → rust → rev → bridge → rev → ios → rev → hacker).

## 11. Owner decisions (2026-10-18)

1. **Resident process: yes, but only while a phone session exists.** The agent starts with the
   first phone connection and exits without one (no always-on process; idle laptop cost stays 0 %
   when no phone is connected). Consequence: "finished / waiting" push works from the first
   connection on, not before.
2. **Push infrastructure: yes**, APNs through the zero-knowledge relay (`scripts/apns-worker`,
   Cloudflare Worker under the owner's account, Apple `.p8` key as a worker secret). Only
   "a request exists / agent finished", never command text, travels through push.
3. **Host targets: macOS only** (launchd, BSD `ps`/`lsof`). Linux is out of scope for now.
4. **Battery:** covered by 1 (agent lives with the phone session).
5. **Distribution:** first embed the host binary in the iOS app and install it over SSH (preview,
   hash, same guarded writer as the hooks); a Homebrew formula is added later.

Order: P0 (typed `ssh-app-host`, replaces the embedded Python) -> P1 (hook + stdio rpc) -> P2
(launchd agent tied to the phone session) -> P3 (push). Each phase follows the dev-chain.
