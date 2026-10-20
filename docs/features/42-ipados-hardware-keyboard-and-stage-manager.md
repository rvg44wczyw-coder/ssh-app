# Feature 42: iPadOS Native Experience, External Hardware Keyboard & Multi-Window

- **Slug**: `ipados-hardware-keyboard-and-stage-manager`
- **Target Release**: Milestone 31 (iPadOS Desktop-Class Productivity)
- **Roles**: `doc-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: No (UI/Windowing/Keyboard event dispatch; no network or crypto changes)
- **Status**: Reviewed: workable, three fixes (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Many developers use an iPad with Apple Magic Keyboard or external mechanical keyboard as their primary mobile development machine.
Currently:
- The UI is designed primarily for a single-screen iPhone viewport.
- On an iPad, the interface leaves large amounts of empty space and lacks multi-column Split View.
- Standard desktop terminal shortcuts (`Cmd+T` for new tab, `Cmd+W` to close tab, `Cmd+1..9` to switch tabs, `Ctrl+C`, `Ctrl+Z`, `Esc`) are not mapped to hardware key commands.
- iPadOS Stage Manager and external display mirroring scale naively rather than rendering full widescreen PTY geometry.

### 1.2 In Scope
- Desktop-class iPadOS layout via SwiftUI `NavigationSplitView`:
  - 3-column / 2-column sidebar: Hosts & Sessions list | Active Terminal / Agent Feed | Project & Git Inspector.
- Stage Manager & Multi-Window support (`WindowGroup` in SwiftUI):
  - Running Session A (e.g. backend Claude Code) in Window 1 and Session B (e.g. frontend test runner) in Window 2 concurrently.
- Hardware Keyboard Commands (`UIKeyCommand`):
  - Tab management: `Cmd+T` (new tab), `Cmd+W` (close), `Cmd+[` / `Cmd+]` (previous/next tab), `Cmd+1`...`Cmd+9`.
  - Terminal signal passthrough: `Ctrl+C` (`SIGINT`), `Ctrl+D` (`EOF`), `Ctrl+Z` (`SIGTSTP`), `Ctrl+L` (`Clear`).
  - View toggling: `Cmd+Shift+T` (Raw Terminal $\leftrightarrow$ Agent Feed), `Cmd+Shift+G` (Git Changes), `Cmd+Shift+F` (File Browser).
- External Display Mode:
  - When an external monitor is plugged in via USB-C, the app can project the full Raw Terminal to the external monitor with native resolution while keeping the Agent Feed and controls on the iPad.
- Pointer hover effects and trackpad two-finger scrolling inside SwiftTerm and Agent Feed.

### 1.3 Out of Scope
- Local compilation tools on the iPad (all build tasks run remotely via the SSH core).

---

## 2. Layout & Multi-Window Architecture

### 2.1 NavigationSplitView Hierarchy

```text
┌─────────────────┬───────────────────────────────┬───────────────────────────────┐
│ Sidebar         │ Primary Content               │ Inspector / Detail            │
│                 │                               │                               │
│ [server.rack]   │ ● Claude Code (tmux: main)    │ Git Changes (8 files)         │
│ macbook-pro     │                               │                               │
│  ├─ dev-chain   │ >_ cargo check --all          │ ┌───────────────────────────┐ │
│  └─ agent-feed  │ [✓] 214 tests passed          │ │ M crates/core/src/lib.rs  │ │
│                 │                               │ │ A ios/Sources/Views.swift │ │
│ [server.rack]   │ ❯ Review the authentication   │ └───────────────────────────┘ │
│ linux-gpu       │   middleware and fix error.   │                               │
│  └─ ollama-run  │                               │ Host Telemetry:               │
│                 │                               │ CPU: 14%  RAM: 6.2 GB         │
├─────────────────┼───────────────────────────────┼───────────────────────────────┤
│ [ + New Tab ]   │ [ Composer: Type or Speak... ]│ [ Commit & Push ]             │
└─────────────────┴───────────────────────────────┴───────────────────────────────┘
```

### 2.2 Hardware Keyboard Command Matrix

| Shortcut | Action | Scope |
| :--- | :--- | :--- |
| `Cmd + T` | Open new tmux tab / session picker | Global |
| `Cmd + W` | Close current tab (prompts if agent is active) | Global |
| `Cmd + 1` .. `Cmd + 9` | Switch directly to tab index 1..9 | Global |
| `Cmd + Shift + T` | Toggle Dual-Mode: Raw Terminal $\leftrightarrow$ Agent Feed | Active Session |
| `Cmd + Shift + G` | Toggle Git Project Inspector Sheet | Active Session |
| `Cmd + Shift + F` | Toggle Remote File Browser | Active Session |
| `Cmd + K` | Clear Terminal Buffer | Raw Terminal |
| `Cmd + Enter` | Send prompt in Agent Feed | Composer |
| `Ctrl + C` | Send byte `0x03` (`SIGINT`) directly to tmux | Terminal / Feed |
| `Ctrl + Z` | Send byte `0x1A` (`SIGTSTP`) directly to tmux | Terminal |

---

## 3. Dynamic PTY Window Geometry Resizing

In iPadOS Stage Manager, window boundaries can change dynamically when the user resizes a floating window:
1. `SwiftTerm.TerminalView` observes frame size changes.
2. New character columns and rows are calculated:
   $$\text{cols} = \lfloor \text{width} / \text{fontWidth} \rfloor, \quad \text{rows} = \lfloor \text{height} / \text{fontHeight} \rfloor$$
3. Calls Rust Core `SshSessionHandle::resize_pty(session_name, cols, rows)`.
4. Rust Core executes `tmux resize-window -t <name> -x <cols> -y <rows>` and sends SSH channel `window_change` packet.
5. Remote applications receive `SIGWINCH` seamlessly without redrawing glitches.

---

## 4. Verification & Test Suite

1. **SwiftUI Layout Tests**:
   - Compact vs Regular horizontal size class transitions.
   - Stage Manager resizing down to minimal window dimensions without clipping.
2. **Key Command Tests**:
   - `UIKeyCommand` interception when physical keyboard is connected.
   - Ensuring `Cmd` modifiers do not leak unwanted characters into active shell panes.

---

## Review (`rev`, 2026-10-20)

- [High] Do not call `tmux resize-window -x -y`: it pins the window size and breaks every other client, including the Mac. `window_change` on the SSH channel is enough.
- [High] Multiple scenes versus "`sceneDidEnterBackground` closes all sockets": one window going to the background would close the other's sessions. Close sockets only when no scene is active.
- [Medium] Two windows attached to the same tmux session fight over its size; allow one window per session.
- [Medium] External display is one line here but is a separate scene with its own lifecycle; specify or defer.

## Binding corrections after `rev` (2026-10-20)

These replace conflicting text above.

1. **No `tmux resize-window -x -y`.** It pins the window size for every client, the Mac included. The SSH `window_change` on the channel is the only resize.
2. **Scenes and sockets.** Sockets close only when no scene is active; one window going to the background does not close another window's sessions.
3. **One window per tmux session.** Two windows attached to the same session would fight over its size, so the second opens the session read-only or is refused with a message.
4. **External display** is a separate scene with its own lifecycle and is deferred out of the first release.
