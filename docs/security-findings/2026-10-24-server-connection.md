# The server's own connection for the server-level screens (2026-10-24)

## Scope

`crates/core/src/session.rs` (`new_exec_only`, `connect_exec_only`), `ios/Sources/SSHApp/Services/ServerConnections.swift`, `TerminalSessionViewModel.connectForServer`, the wiring in `MainTerminalView` and `SessionsHomeView`. Feature 54, owner's go-ahead 2026-10-24.

Applied: MITM / identity (a new connection that nobody watches), credential handling, DoS, battery. Ruled out: parsing (no new input format; the same host calls as a session).

Live tests: `a_server_handle_runs_commands_without_a_session_and_refuses_an_unpinned_key` against a loopback sshd (no pin and an empty pin refused with no connection made, a wrong pin refused as `HostKeyChanged`, the right pin runs commands, a second connect keeps the live connection, `disconnect` leaves nothing that can run a command, a new connect works again); tmux server pid unchanged (35350). Swift tests: a server without a pinned host key cannot be opened, a server model without a pin fails without creating a handle.

## Design

- One exec-only handle per server (the same `SshSessionHandle`, no terminal, no tmux). The host key must be pinned: the core refuses before it opens a socket. Nobody sees a terminal here, so a key is never trusted on first use.
- The connection exists only while a screen of the server is open: closed `15 s` after the last one closes and at once in the background (`ServerConnections.closeAll`); a screen still open reconnects on return.
- A server with a live session keeps using that session's channel (no second connection).
- The Sessions screen never opens a connection by itself: nothing connects to a server just because the list is on screen; the runtime probe stays for servers with a session.

## Findings

1. **Low, handled**: a failed open (wrong pin, offline) leaves a holder count; the caller releases it on failure so the idle close still runs.
2. **Controversial, kept**: the buttons of a server without a session are enabled whenever its key and pin exist, even if the host is off; the tap then ends in an alert with the reason. Probing every server on screen would be a connection per server per visit, which the battery rule forbids.
3. **Controversial, kept**: a changed host key shows the same error as a session's (both fingerprints named); the screen offers no "trust the new key" button, that stays in the session flow.

## Verdict

No open finding. Device run pending (the sheet opening over a server with no tab, the background close, the return).
