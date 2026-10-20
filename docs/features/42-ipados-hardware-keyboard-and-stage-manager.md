# Feature 42: iPadOS Native Experience, External Hardware Keyboard & Multi-Window

- **Slug**: `ipados-hardware-keyboard-and-stage-manager`
- **Target Release**: Milestone 31 (iPadOS Desktop-Class Productivity)
- **Roles**: `doc-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: No (UI/Windowing/Keyboard event dispatch; no network or crypto changes)
- **Status**: Reviewed and corrected (`rev`, 2026-10-20).

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
  - Running Session A in Window 1 and Session B in Window 2 concurrently. **One window per tmux session**: two windows attached to the same session would fight over its size, so a second window on the same session opens it read-only or is refused with a message.
  - Sockets close only when **no scene is active**; one window going to the background does not close the other window's sessions.
- Hardware Keyboard Commands (`UIKeyCommand`):
  - Tab management: `Cmd+T` (new tab), `Cmd+W` (close), `Cmd+[` / `Cmd+]` (previous/next tab), `Cmd+1`...`Cmd+9`.
  - Terminal signal passthrough: `Ctrl+C` (`SIGINT`), `Ctrl+D` (`EOF`), `Ctrl+Z` (`SIGTSTP`), `Ctrl+L` (`Clear`).
  - View toggling: `Cmd+Shift+T` (Raw Terminal $\leftrightarrow$ Agent Feed), `Cmd+Shift+G` (Git Changes), `Cmd+Shift+F` (File Browser).
- Pointer hover effects and trackpad two-finger scrolling inside SwiftTerm and Agent Feed.

### 1.3 Out of Scope
- External display mode: it is a separate scene with its own lifecycle and is deferred to its own spec.
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
4. Rust Core sends the SSH channel `window_change` packet. It does **not** call `tmux resize-window -x -y`: that pins the window size for every client, the Mac included; tmux sizes the window from the attached clients.
5. Remote applications receive `SIGWINCH` seamlessly without redrawing glitches.

---

## 4. Verification & Test Suite

1. **SwiftUI Layout Tests**:
   - Compact vs Regular horizontal size class transitions.
   - Stage Manager resizing down to minimal window dimensions without clipping.
2. **Key Command Tests**:
   - `UIKeyCommand` interception when physical keyboard is connected.
   - Ensuring `Cmd` modifiers do not leak unwanted characters into active shell panes.
3. **Multi-window tests**:
   - Backgrounding one scene keeps the other scene's socket open; backgrounding the last active scene closes all.
   - A second window on an already open session is refused or read-only.
   - No `resize-window` command is ever issued (a test on the command builder).

---

## Review (`rev`, 2026-10-20) - applied

Folded in above: no `resize-window`; sockets close when no scene is active; one window per session; external display deferred.
