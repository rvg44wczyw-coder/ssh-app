# Feature 27: live detection of agents in tmux (agent, phase P2 follow-up)

Status: implemented (2026-10-18). Follows `26-host-agent.md`.

## 1. Goal

Agents started in tmux without hooks (or before the hooks were installed) must also produce live
state events, not only appear in a one-off `state` call.

## 2. Decision: no `tmux -C` control client

The first idea was a `tmux -C attach` client in the agent that reacts to `%sessions-changed`.
Rejected: a control client attaches to the user's own tmux server, shows the session as
`(attached)` in `tmux ls`, appears in `list-clients` and can interfere with `detach-on-destroy`
and `attach -d`. The laptop is also the SSH host and the user's working environment (see the two
tmux incidents in AGENTS.md), so the agent must stay a read-only observer of tmux.

## 3. Design

- While at least one phone is subscribed, the agent re-checks the agents found in tmux every 15 s
  (`LIVE_RESCAN`; test override `SSH_APP_LIVE_RESCAN_MS`). Without a subscriber it keeps the
  600 s rescan and the kqueue wake-ups, so an idle agent still uses no CPU.
- The state that is diffed is now the same as the one `state` returns: hook records, pending
  files and the tmux-detected sessions from the cached usage scan (`merge_detected`). A detected
  session that appears or disappears is reported as `none -> running` / `running -> gone`.
- The scan itself is the existing stale-while-revalidate usage report; no new tmux command and
  nothing is written to tmux.

## 4. Tests

`crates/host/tests/agent.rs::a_subscriber_gets_events_for_agents_that_appear_in_tmux`: a
subscriber, then a fake `claude` in a private-socket tmux, expects `none -> running` for it.
