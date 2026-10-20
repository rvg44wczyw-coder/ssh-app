# Feature 49: Zellij Terminal Multiplexer Support & Dual-Mux Architecture

- **Slug**: `zellij-multiplexer-support`
- **Target Release**: Milestone 38 (Next-Gen Terminal Multiplexer Parity)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (session name sanitization, layout file parsing, WASM plugin boundaries)
- **Status**: Reviewed and rescoped (`rev` + `hacker`, 2026-10-20): milestone A is "attach and detach only"; feed parity is a separate milestone B that waits on a scriptable Zellij screen dump. Implementation pending.

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

**Milestone B: feed parity**, only if Zellij gains a scriptable way to dump a pane's text and send keys. Not scheduled. The earlier estimate covered milestone A only.

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
