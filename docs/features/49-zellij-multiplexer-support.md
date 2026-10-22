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

Checked on **Zellij 0.45.1** against a private socket folder, from outside the session:

- Screen: `zellij --session NAME action dump-screen --full --ansi` prints the **focused** pane with scrollback and styles (like `tmux capture-pane -e -p -J -S -`). If a plugin pane has focus the dump is empty; the feed then shows nothing new and the terminal stays usable. `--pane-id` exists but the feed follows focus, like the user.
- Keys: the phone already types through its own PTY, not through `send-keys`, so answering prompts, Enter and Ctrl-C need nothing new.
- No pane pid exists (`list-panes --json` has `pane_command` and `pane_cwd` only). An agent in a pane is found instead by the environment every process in a Zellij pane carries: `ZELLIJ_SESSION_NAME=<name>` (and `ZELLIJ_PANE_ID`). `ps -Eww` (macOS) / `ps eww` (Linux) show it for the user's own processes; a SIP-protected system binary hides its environment, a node/python agent does not (checked).

### What changes

| Part | Change |
| :--- | :--- |
| Screen capture (`host_api`) | `capture_session_screen_in(name, known_sum, kind)`; tmux unchanged; Zellij runs the dump above, same checksum/`SAME` protocol, same 3 MB bound. |
| Pane context and transcript detection | `detect_pane_context_in` / `detect_session_transcript_path_in(name, kind)`: for Zellij the root set of processes is "those whose environment has `ZELLIJ_SESSION_NAME=<name>`" instead of "children of `pane_pid`"; the rest of the script (agent recognition, cwd, session file) is shared. |
| Hook registry (Python hook and Rust port, `sessions.rs`) | The `tmux` field is the multiplexer session name: when `$TMUX` is empty and `$ZELLIJ` is set it holds `$ZELLIJ_SESSION_NAME`. A tmux and a Zellij session with the same name share a key; both are the same user's, and the app names its tabs uniquely per server. |
| Host screen watch (`screenwatch::capture`) | tries tmux, then Zellij's dump for the same name. |
| iOS | `isTerminalOnly` goes away for Zellij; chat mode is allowed; the calls pass `multiplexer`. Restart-agent (`respawn-pane`) has no Zellij form: typed `Unsupported`, the button is hidden. |

### Security (hacker gate)

| Threat | Mitigation |
| :--- | :--- |
| Name injection into the dump/scan command | Same `validate_zellij_name` as attach; the name reaches the shell single-quoted and the awk program as a `-v` variable, never inside the program text. |
| Environment of other processes leaks | The scan runs on the host and only pids and a yes/no match leave awk; no environment text is printed or sent. |
| A hostile process fakes `ZELLIJ_SESSION_NAME` | It is the same user's process on the user's own host; the worst case is that the feed shows another of the user's agents. Accepted, as the tmux pane-pid walk has the same trust. |
| Screen text is untrusted | Bounded and parsed exactly like tmux's capture. |
| Tests touching the developer's Zellij | Private `ZELLIJ_SOCKET_DIR`/`XDG_RUNTIME_DIR`/`HOME`, `ZELLIJ`, `ZELLIJ_SESSION_NAME` removed. |
