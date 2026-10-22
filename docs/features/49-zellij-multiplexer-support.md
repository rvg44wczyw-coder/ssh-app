# Feature 49: Zellij Terminal Multiplexer Support & Dual-Mux Architecture

- **Slug**: `zellij-multiplexer-support`
- **Target Release**: Milestone 38 (Next-Gen Terminal Multiplexer Parity)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (session name sanitization, layout file parsing, WASM plugin boundaries)
- **Status**: Reviewed and rescoped (`rev` + `hacker`, 2026-10-20): milestone A is "attach and detach only"; feed parity is a separate milestone B that waits on a scriptable Zellij screen dump. Milestone A implemented 2026-10-22 (see the end of this file).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
While `tmux` has been the default standard for decades, a large and rapidly growing segment of modern developers (especially in Rust, Go, and WebAssembly ecosystems) uses **Zellij**.
Zellij provides:
- Modern YAML/KDL layouts, floating panes, responsive status bars, and WebAssembly plugins out of the box.
- Developers whose remote servers are configured around Zellij currently cannot use `ssh-app`'s session persistence without installing and configuring `tmux`.

### 1.2 In Scope

**Milestone A: attach and detach only.** A Zellij session is a terminal tab that survives a disconnect. It has **no** agent feed, no screen watch, no answering agents from the phone, no scrollback preview and no per-session approvals: those rely on `tmux capture-pane`, `send-keys` and `list-sessions`, and Zellij has no equivalent set. The UI says so on the session card ("Zellij: terminal only").

1. **Extend the existing `MultiplexerKind` in `crates/core/src/multiplexer.rs`** (it already has `Tmux` and `WindowsMux` and builders for attach-or-create, detach, kill and list) with a `Zellij` variant. There is no new `mux.rs` and no new trait; `crates/mux` is the unrelated native-Windows multiplexer (feature 34).
2. **Detection** on the host by `command -v zellij` and `command -v tmux` (the helper `doctor` reports both with versions). Priority is a per-server choice: Auto / Prefer Zellij / Prefer tmux; the default is tmux when both exist.
3. **Zellij commands**, each **verified against the Zellij release the host reports** before it is written into the builder (the CLI changed between versions; where a flag is missing the builder returns a typed "unsupported on this Zellij version"):
   - create/attach: `zellij attach --create <name>` (the exact spelling and layout option are fixed after the check);
   - list: `zellij list-sessions` with `--no-formatting` only where supported, parsed defensively;
   - kill: `zellij kill-session <name>` / `delete-session <name>` (with `--force` only where supported).
4. **Resize** is the SSH `window_change`; Zellij reacts to `SIGWINCH`. TrueColor works natively.

**Milestone B: feed parity.** Zellij 0.45 gained `action dump-screen`, `send-keys`, `write-chars`, `paste`, `list-panes --json`, so the condition set for B is met. Design in the last section. Status: in work 2026-10-22.

### 1.3 Out of Scope
- Re-implementing the Zellij server in Rust Core (we manage the remote `zellij` binary over SSH, exactly as we manage `tmux`).

---

## 2. Architecture (`crates/core/src/multiplexer.rs`)

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum MultiplexerKind {
    #[default]
    Tmux,
    WindowsMux,
    Zellij, // new: attach, detach, kill, list only
}
```

The four builders (`attach_or_create`, `detach`, `kill`, `list_detailed`) gain a `Zellij` arm. The session list for Zellij carries only name and attached flag; fields the tmux list has (pane count, preview, agent state) are absent and the UI hides them. Every feature that needs a screen dump or key injection checks `kind.supports_screen_access()` (true for `Tmux`, false for the others) and is greyed out with the reason.

---

## 3. Security & Threat Modeling (Hacker Gate)

| Threat Vector | Mitigation Strategy |
| :--- | :--- |
| **Session name injection** | Names pass the existing `validate_session_name` and, for Zellij, `^[a-zA-Z0-9_-]{1,64}$`. Names are rejected, not rewritten, so two different names cannot collapse into one. The name is passed through the same shell quoting as for tmux. |
| **Layouts and plugins** | The app never passes a layout or plugin path. A layout is used only by name from `$HOME/.config/zellij/layouts`, and the app does not write there. Zellij WASM plugins run with the user's rights; the spec does not add any. |
| **Zellij flags** | Only the fixed flags in section 1.2 are ever built; nothing from the user reaches the Zellij argv except the validated name. |
| **Test isolation** | A loopback test must not touch the developer's Zellij: private `ZELLIJ_SOCKET_DIR` (or `XDG_RUNTIME_DIR`) in a 0700 folder, `ZELLIJ`, `ZELLIJ_SESSION_NAME` removed, same rule as tmux in AGENTS.md. |

---

## 4. Verification & Test Suite

1. **Rust Core Unit Tests**:
   - `test_detect_zellij_installed_probe`.
   - `test_parse_zellij_list_sessions_output`: parsing standard tabular output of `zellij list-sessions`.
   - `test_zellij_session_name_is_rejected_not_rewritten`: `a.b`, `a:b`, `a b`, 65 characters are refused.
   - `test_unsupported_flag_is_a_typed_error`.
   - `test_screen_features_are_off_for_zellij`.
2. **Loopback sshd Integration Test**:
   - Only where `zellij` is installed (the test skips with a message otherwise): spawn, list, detach and kill a session through the builders, with the private socket directory above.

---

## Review (`rev`, 2026-10-20) - applied

Folded in above: honest scope (attach/detach only), reuse of the existing `multiplexer.rs`, CLI flags verified per release, work split into milestones A and B, test isolation for Zellij.

## Built 2026-10-22 (milestone A)

CLI checked against **Zellij 0.45.1** on a private socket folder: `attach --create <name> [-- <command>]` (the command only matters when the session is created), `list-sessions --no-formatting` prints `name [Created 2s ago]` (dead, resurrectable sessions say `EXITED` and are left out of the list), `delete-session --force` kills and forgets a session (older releases: kill then delete, the builder falls back), there is **no** way to detach another client from outside, so `detach` is the typed error `Unsupported`.

- `crates/core/src/multiplexer.rs`: `MultiplexerKind::Zellij`, `MultiplexerPreference` (Auto / PreferZellij / PreferTmux), `multiplexer_probe_command` + `parse_multiplexer_probe` + `choose_multiplexer` (Auto: tmux when both exist), `supports_screen_access()` (tmux only), names `[A-Za-z0-9_-]{1,64}` refused rather than rewritten, start folder validated like tmux's, the initial command single-quoted and run by the login shell. `detect_multiplexer_once(config, preference)` asks the host over one short connection; any failure means tmux.
- Tests: names, attach text, typed unsupported, probe parsing and choice, the probe command run for real, and list/kill against a real private Zellij server (skipped where `zellij` is missing); the developer's own sessions are never reachable (`ZELLIJ_SOCKET_DIR`, `ZELLIJ`, `ZELLIJ_SESSION_NAME`, `TMUX` removed).
- iOS: server sheet gets "Session program" (Auto / Prefer tmux / Prefer Zellij, kept per `user@host:port`), the chosen program is asked once per host and app run, a Zellij tab shows "Zellij: terminal only" and cannot switch to the feed. Other tmux-only features (screen watch, agent keys) find no tmux session and report that; greying them out one by one is left to milestone B.
- Not tried on a device or with a Zellij server over real SSH (only the builders and a local server).

Security (hacker gate): no user text reaches the Zellij argv except the validated name; no layout or plugin option is ever built; the only quoting of free text is the initial command (single-quoted, escaped, run by the user's login shell like tmux's); tests never touch the developer's Zellij. Nothing found.


## Milestone B design (2026-10-22): the agent feed on Zellij

Checked on **Zellij 0.45.1** against a private socket folder, from outside the session. Needs a Zellij with `action list-panes` and `dump-screen --pane-id`; the version comes from the probe of milestone A, **0.45 is the minimum until older releases are checked**. Below it, or when the probe has no version, the feed is unavailable and the tab stays a terminal (the session strip says why).

### Facts used

- **The pane**: `zellij --session NAME action list-panes` prints a table; the last three tokens of a row are `FOCUSED FLOATING EXITED` and the first is the pane id (`terminal_N` / `plugin_N`). The tab's **focused terminal pane** is the first `terminal_N` row whose third-from-last token is `true`. (A focused plugin pane, such as the status bar wizard, does not hide it.) This is the equivalent of tmux's active pane.
- **The screen**: `zellij --session NAME action dump-screen --pane-id terminal_N --full --ansi` prints that pane with scrollback and styles, like `tmux capture-pane -e -p -J -S -`. If `--ansi` is refused by an older release the call is repeated once without it.
- **Keys** need nothing new: the phone types through its own PTY, so answers, Enter and Ctrl-C already work.
- **No pane pid exists.** The agent in the pane is found by the environment all its processes carry, `ZELLIJ_SESSION_NAME=<name>` and `ZELLIJ_PANE_ID=<n>`. macOS: `ps -Eww -Ao pid=,command=`; Linux: `/proc/<pid>/environ` (**not tried on Linux yet**). A SIP-protected system binary hides its environment, a node/python agent does not (checked with a `node` process). The scan runs on the host and prints pids only.
- **An unknown session name** makes Zellij print `Session 'x' not found. The following sessions are active: ...` to stderr and exit 1; it creates nothing. stderr is discarded so other session names never reach the app.

### Interface (`crates/core`, exposed over UniFFI)

```rust
// host_api, on SshSessionHandle; the old tmux-only methods stay and call these with Tmux.
pub async fn capture_session_screen_in(&self, session_name: String, known_sum: Option<String>, kind: MultiplexerKind) -> Result<ScreenCapture, SshCoreError>;
pub async fn detect_pane_context_in(&self, session_name: String, kind: MultiplexerKind) -> Result<PaneContext, SshCoreError>;
pub async fn detect_session_transcript_path_in(&self, session_name: String, kind: MultiplexerKind) -> Result<Option<String>, SshCoreError>;
// WindowsMux -> SshCoreError::Unsupported; names are validated per kind (tmux rules / validate_zellij_name).
// restart_session_command for Zellij -> SshCoreError::Unsupported (no respawn-pane equivalent).
```

`MultiplexerKind::supports_screen_access()` becomes true for `Tmux` and `Zellij`.

### What changes

| Part | Change |
| :--- | :--- |
| Screen capture | as above; same checksum / `SAME` protocol and the 3 MB bound. |
| Pane context and transcript detection | the roots of the process walk are "the pids whose environment has this session name and pane id" instead of "the pane pid"; the agent recognition, folder and session file logic is shared with tmux. For the transcript the hook registry stays first (exact session id), the process scan second. |
| Hook registry (Python hook and Rust port) | the `tmux` field is the multiplexer session name: when `$TMUX` is empty and `$ZELLIJ` is set it holds `$ZELLIJ_SESSION_NAME` (cleaned like the tmux name). |
| Host screen watch | `screenwatch::capture` tries tmux, then the Zellij dump of the focused pane for the same name. |
| iOS | `isTerminalOnly` is true only while the feed is unavailable (Zellij below the minimum version); chat mode and the feed work on Zellij; the calls pass `multiplexer`. The "Restart agent" action is hidden for Zellij tabs. |

### Named tradeoffs

- The environment scan is a trick, not an API: it needs `ps` to show the environment. It is used because nothing else tells which process runs in a Zellij pane; the registry stays the first source for the transcript, so a host where the scan fails still gets the exact file for any agent with hooks.
- A shared `tmux` field means a tmux and a Zellij session with the same name on one host share a registry key. The app names its tabs uniquely per server and both sessions belong to the same user, so the worst case is a card for the other of the user's own sessions. A separate `mux` field would touch every reader and was not worth it.

### Security (hacker gate)

| Threat | Mitigation |
| :--- | :--- |
| Name injection into the dump/scan commands | `validate_zellij_name`; the name reaches the shell single-quoted and awk only as a `-v` variable. |
| Environment of other processes leaks | Only pids and match results leave awk; no environment text is printed or sent. |
| A process fakes `ZELLIJ_SESSION_NAME` | Same user, same host; the worst case is another of the user's agents in the feed. |
| Other session names leak through Zellij's error text | stderr is discarded. |
| Screen text is untrusted | Bounded and parsed exactly like tmux's capture. |
| Tests touching the developer's Zellij | Private `ZELLIJ_SOCKET_DIR`/`XDG_RUNTIME_DIR`/`HOME`, `ZELLIJ`, `ZELLIJ_SESSION_NAME` removed. |

### Tests

1. Unit: the focused-pane awk on fixture tables (plugin focused, two terminals, floating, none); script text for both kinds; name validation per kind; `WindowsMux` and restart give `Unsupported`; the registry name rule (tmux wins, then Zellij, else empty) in Python and Rust.
2. Loopback with a real private Zellij (skipped where `zellij` is missing): type a line, the capture returns it and answers `SAME` the second time; the pane context finds a fake agent (a `node` script named `claude`) in the focused pane and not one in another session; an unknown session is an error and creates nothing.
3. Swift: the view model passes the multiplexer; a Zellij tab can switch to chat mode when the feed is available.
