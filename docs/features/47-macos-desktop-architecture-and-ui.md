# Feature 47: Native macOS Terminal User Interface (TUI) via Ratatui & Crossterm

- **Slug**: `macos-tui-architecture`
- **Target Release**: Milestone 36 (Desktop-Class Terminal User Interface in Pure Rust)
- **Roles**: `doc-dev`, `rust-core-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (raw terminal mode, panic hook recovery, secure key storage, terminal escape sanitization)
- **Status**: Reviewed and corrected (`rev` + `hacker`, 2026-10-20); implementation pending. It is a new product; its first deliverable is a spike (section 6). There is no `rust-tui-dev` skill: the work follows `rust-core-dev` and the TUI rules of section 3.

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Developers and power users on macOS spend their entire day inside terminal emulators (Ghostty, Alacritty, Kitty, WezTerm, iTerm2, Terminal.app).
Forcing them to open a heavy GUI or electron window disrupts their flow:
- They want a **keyboard-driven, lightning-fast Terminal User Interface (TUI)** that, as a goal to be measured (start under about 5 ms, under about 15 MB, no number promised before it is measured), and can be controlled entirely via Vim keys (`h`, `j`, `k`, `l`, `/`, Tab, Esc).
- The TUI must provide all the superpowers of the iOS app:
  1. Multi-server and multi-agent monitoring (Claude Code, Antigravity, Ollama).
  2. One-tap/keystroke cryptographic SSHSIG approvals with live countdown.
  3. Docker container monitoring, logs, and in-container shell attach.
  4. Git project inspector with hunk staging.
  5. Structured Agent Feed with syntax highlighting and collapsible thinking blocks.
- It must run natively on macOS (Apple Silicon & Intel) and can even be invoked remotely over SSH.

### 1.2 In Scope
1. **New Crate `crates/tui` in Workspace**:
   - Built on **Ratatui** (0.28+) and **Crossterm** (0.28+).
   - Async runtime powered by `tokio`, consuming the existing `crates/core` library directly (no UniFFI or C overhead).
   - Generates the standalone binary `ssh-app-tui` (crate `crates/tui`), installed with `cargo install --path crates/tui`. A Homebrew tap does not exist yet; creating it is a later task and is not referred to as available.
2. **3-Pane Adaptive TUI Layout**:
   - **Left Panel [1]**: Servers, Workspaces, Docker Containers, Agent Sessions.
   - **Center Panel [2]**: Dual-mode: Raw PTY Terminal $\leftrightarrow$ Structured Agent Feed (formatted Markdown, code blocks, diffs).
   - **Right Panel [3]**: Real-time telemetry sparklines/gauges (CPU, RAM, GPU, Disk), Git status & hunk staging.
   - **Bottom Line**: Status bar, server ping, lease countdown, pending approvals badge, hotkey cheat sheet.
3. **Modal Popups (Floating TUI Dialogs)**:
   - **Approval Card Modal**: Centered popover with countdown timer, command risk analysis badge, and `[y] Approve / [n] Deny` keys.
   - **Docker Management Modal**: Container status, memory usage, `[r] Restart`, `[l] Logs`, `[t] Terminal`.
   - **Quick Command Palette (`:`)**: Fuzzy search actions and servers (like Neovim / fzf).
4. **Vim / Kakoune Keyboard Navigation**:
   - `Tab` / `Shift+Tab`: Cycle active panels (Sidebar $\rightarrow$ Feed/Terminal $\rightarrow$ Inspector).
   - `1`, `2`, `3`: Jump directly to panel 1, 2, or 3.
   - `j` / `k` or Arrow Keys: Navigate lists and scroll terminal buffers.
   - `a`: Quick approve pending request; `d`: Deny.
   - `c`: Focus composer to speak/type to agent.
   - `q`: Quit cleanly (or background daemon).

### 1.3 Out of Scope
- GUI Cocoa / AppKit windows (the desktop app is 100% TUI).

---

## 2. TUI Layout & Wireframes (Strict No-Emoji Design System)

### 2.1 Full-Screen 3-Pane Ratatui Layout

```text
┌─ [1] SERVERS & SESSIONS ───────┬─ [2] AGENT FEED: macbook-pro/claude ────────┬─ [3] PROJECT & TELEMETRY ─────┐
│ ▼ macbook-pro                  │ ● Claude Code  Session: claude-refactor     │ Git: main [3 modified]        │
│   ● claude-refactor (Active)   │ ─────────────────────────────────────────── │ ┌───────────────────────────┐ │
│   ○ agy-architecture (Idle)    │ ❯ cargo test --lib                          │ │ M crates/core/src/auth.rs   │ │
│   >_ term-1 (shell)            │ [✓] 254 tests passed in 1.42s               │ │ M crates/tui/src/app.rs     │ │
│                                │                                             │ └───────────────────────────┘ │
│ ▼ Docker Containers (3)        │ ⎿ Tool: Edit `crates/core/src/auth.rs`      │ [s] Stage  [u] Unstage  [c] Commit │
│   ● postgres-16     148MB      │ + pub fn verify_bearer_token(...)           │ ───────────────────────────── │
│   ● ollama-runner   4.8GB      │ - pub fn verify_token(...)                  │ Telemetry:                    │
│   ● redis           12MB       │                                             │ CPU [||||||||||||        ] 28%│
│                                │ <thinking: 42 lines collapsed> [Enter view] │ RAM [||||||||||||||||||  ] 7.2G│
│ ▼ gpu-box-linux                │                                             │ GPU [||||||||            ] 42%│
│   ⬡ deepseek-coder  (Running)  │ ─────────────────────────────────────────── │ Temp: 54°C (RTX 4090)         │
│                                │ >_ (Composer): Type instruction...          │ Host: Darwin arm64 (M3 Max)   │
├────────────────────────────────┴─────────────────────────────────────────────┴───────────────────────────────┤
│ [F1] Help  [Tab] Switch Panel  [a] Approve (1 pending!)  [d] Docker  [:] Command Palette  [q] Quit           │
└───────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Floating Modal: Cryptographic Approval Card (`ratatui::widgets::Clear`)

When an agent needs permission, a high-priority modal overlay appears centered on screen:

```text
┌─────────────────────────── PENDING APPROVAL (01:42) ───────────────────────────┐
│ Host: macbook-pro          Agent: Claude Code               Tool: run_command  │
│ Risk: ELEVATED (Modifies filesystem outside sandbox)                           │
├────────────────────────────────────────────────────────────────────────────────┤
│ Command:                                                                       │
│   git reset --hard HEAD~1 && cargo clean                                       │
│                                                                                │
│ Analysis:                                                                      │
│   [!] Destructive command: resets uncommitted changes in git worktree.         │
│   [!] Disk space reclamation: purges target/ cache directory.                  │
├────────────────────────────────────────────────────────────────────────────────┤
│ [y] Sign & Approve (SSHSIG)         [n] Deny Request         [e] Edit Command  │
└────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Architecture & Event Loop (`crates/tui`)

### 3.1 Crate Structure

```text
crates/tui/
├── Cargo.toml
└── src/
    ├── main.rs              # Entry point, terminal initialization & panic hook
    ├── app.rs               # App state machine, active panels, focus manager
    ├── event.rs             # Crossterm event stream (Key, Mouse, Resize, Tick)
    ├── ui/
    │   ├── mod.rs           # Ratatui layout orchestrator
    │   ├── sidebar.rs       # Server, session & docker tree widgets
    │   ├── feed.rs          # Agent feed renderer (markdown parser, syntax highlighter)
    │   ├── inspector.rs     # Telemetry sparklines & git diff view
    │   ├── modals/
    │   │   ├── approval.rs  # Approval card modal
    │   │   ├── docker.rs    # Container logs & restart manager
    │   │   └── palette.rs   # Fuzzy command runner
    │   └── statusbar.rs     # Bottom status strip
    └── handler.rs           # Keybinding dispatcher
```

### 3.2 Terminal Lifecycle & Panic Safety

```rust
pub fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, std::io::Error> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    
    // Safety: Set custom panic hook to always restore terminal state on panic
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture, crossterm::cursor::Show);
        default_hook(panic_info);
    }));

    Terminal::new(CrosstermBackend::new(stdout))
}

pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), std::io::Error> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}
```

---

## 4. Security & Threat Modeling (Hacker Gate)

| Threat Vector | Mitigation Strategy |
| :--- | :--- |
| **Terminal Raw Mode Lockup (Frozen Terminal)** | Custom `std::panic::set_hook` disables raw mode, leaves the alternate screen, disables mouse capture **and shows the cursor again** (the hook in 3.2 does all four), even on panics in other threads; a drop guard does the same on normal early returns. |
| **SSHSIG Approval Key Security** | **The Mac has its own key.** The phone's private key never leaves the phone. The TUI generates its own Ed25519 key (macOS Keychain, or a `0600` file under `~/.ssh-app/`), registered in the host's `allowed_signers`/`authorized_keys` through the pairing path of feature 44, and approvals from the Mac are signed with it. Keys are zeroized on drop. |
| **ANSI Escape Bombing via Agent Feed** | All incoming text streams go through `strip_ansi_codes` (`crates/core/src/transcript.rs`) or are tokenized into `Ratatui::Span` widgets; a test feeds OSC 52 (clipboard), title-change and cursor-move sequences and asserts that no `0x1b` byte reaches the backend. Raw terminal control sequences (`\x1b]...`) cannot escape into the host terminal. |
| **Background Zombie Processes** | When the TUI exits (`q`), all open SSH exec channels are closed cleanly. Background tmux sessions continue on remote servers without orphan client processes. |

---

## 5. Verification & Test Suite

1. **Rust Core & TUI Tests**:
   - `test_tui_app_state_panel_cycling`: Tab key transitions between Sidebar, Feed, and Inspector.
   - `test_approval_modal_key_dispatch`: `y` signs and submits approval; `n` sends deny.
   - `test_markdown_feed_line_wrapping`: tests wrap-around on narrow 80-column terminals.
   - `test_crossterm_panic_hook_cleanup`: verifies terminal restoration logic.
2. **Interactive TUI Smoke Test**:
   - Run `cargo run -p ssh-app-tui` in Ghostty / Alacritty / Terminal.app.
   - Verify layout rendering, modal popups, and smooth scrolling.

---

## 6. What is reused from `crates/core`, and the first spike

Reuse is a list, not a claim. What the TUI calls:

| TUI part | `crates/core` it calls | Note |
| :--- | :--- | :--- |
| Connect, host-key check | `transport` | native Rust, no UniFFI |
| Session list, attach | `tmux`, `multiplexer` | |
| Agent feed | `screen` (parser of the screen), `transcript` | the screen text arrives through `capture-pane` over SSH, so a feed needs a live SSH connection, as on the phone |
| Approvals | `approval`, `approval_queue`, `host_api` | signed with the Mac's own key (section 4) |
| Docker | feature 45 (not built yet) | the Docker panel is off until it exists |
| Git panel | `gitstatus`, `githunks`, `gitindex` | |

**Spike (first deliverable, before any layout work):** connect to one host from the TUI, show the session list, attach to one session, and approve one request signed by the Mac's key. Measure start time and memory then, and replace the goals in section 1.1 with the numbers.

## Review (`rev`, 2026-10-20) - applied

Folded in above: the Mac has its own key; numbers are goals; the `rust-tui-dev` role is dropped; one name (`ssh-app-tui`, `crates/tui`) and no existing tap; the reuse table with the `capture-pane` dependency; the panic hook restores the cursor.
