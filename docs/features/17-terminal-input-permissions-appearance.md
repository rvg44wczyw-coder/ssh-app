# 17. Terminal scroll/click, quick commands, local chat, session permissions, appearance

Status: implemented (Rust + iOS); on-device checks pending (scroll feel, alternate icons, config-dir detection).

## 1. Scroll and click through tmux
- tmux sends an attach client only the visible screen; history stays in tmux. The attach command sets `mouse on` for the session.
- A vertical pan becomes SGR mouse-wheel events (`ESC[<64;x;yM` up, `65` down), one step per text row, ×5 speed, CADisplayLink momentum. Wheel-up enters copy-mode, wheel-down at the bottom leaves it.
- A single tap sends SGR press+release (`ESC[<0;c;rM` / `m`) at the tapped cell, so "Jump to bottom" buttons in Claude/agy work. Dragging also dismisses the keyboard.
- Tests: loopback sshd (wheel scroll, tap click to a mouse-aware program) on a private tmux socket.

## 2. Keyboard accessory and quick commands
- The system bar already has Esc/Ctrl/Tab/arrows, so ours has: assistant, quick action, ^C, y/n, user commands, editor.
- User commands (`QuickCommand`, editor `QuickCommandsEditorView`); defaults: clear, /clear, /model, /help, /compact, /status.

## 3. Local chat assistant
- Chat mode next to Command mode in the AI assistant sheet; host Ollama `/api/chat` over SSH.
- Caps: 60 messages / 96 KB; typed errors; Task cancellation drops the host command.
- Visibility setting: Auto / Always / Never (`ai_assistant_visibility`).

## 4. Session permissions
- Menu tile "Permissions": folder of the session, config folder (detected from the running Claude's `CLAUDE_CONFIG_DIR` via `ps eww`), start mode, user + project (shared/local) rules, suggestions from what is installed.
- Launch profile `permission_mode` → `--permission-mode` (Claude) / `--mode` (agy). Template `project-work`.
- Editor reads with `readAgentPermissionsChecked`: invalid rules (e.g. control characters) are shown escaped and removed only after confirmation.
- Antigravity's own command allowlist is internal and not writable.

## 5. Appearance (Settings → Appearance)
- Accent: 12 presets or a custom color (`#rrggbb` in `app_accent`); applied via `Color.appAccent` and root `.tint`.
- Terminal color schemes: 17 built-in + user schemes. Defaults per slot (general, Claude, Antigravity, Codex, Grok, shell), per-profile override. Resolution: profile → slot → general → built-in pick.
- Scheme editor: name, background, text, cursor, 16 ANSI colors, preview, "start from"; custom schemes in UserDefaults (`terminal_custom_schemes`, ids `custom-*`).
- Icons: per-profile symbol + color (tabs, profile list); home-screen app icon (classic, 7 tints, light) through alternate icon sets.

## Open
- Per-tab quick scheme switch and `/theme` in the Commands palette.
- Real subscription quota has no safe source.
