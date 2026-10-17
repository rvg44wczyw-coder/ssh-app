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
- Terminal color schemes: 18 built-in + user schemes. Defaults per slot (general, Claude, Antigravity, Codex, Grok, shell), per-profile override. Resolution: profile → slot → general → built-in pick.
- Scheme editor: name, background, text, cursor, 16 ANSI colors, preview, "start from"; custom schemes in UserDefaults (`terminal_custom_schemes`, ids `custom-*`).
- Icons: per-profile symbol + color (tabs, profile list). The stored value is an SF Symbol name, `glyph:<text>` (the 9 terminal glyphs from the sheet in `docs/icons`: `>_ $ ~ # ▮ > [] — %`) or `img:<file>` (the user's own picture, chosen with the photo picker, cropped to a square, saved as 96 px PNG in Application Support/ProfileIcons, replaced pictures are deleted).
- Home-screen app icon: classic, 7 tints, light, and the 5 designs from `docs/icons` (grid minimal / prompts / rounded / symbols / bold) through alternate icon sets (`AppIcon-<Name>`, previews `IconPreview-<Name>`). iOS only accepts icons bundled with the app, so a user-uploaded picture cannot be a home-screen icon.

## Open
- Per-tab quick scheme switch and `/theme` in the Commands palette.
- Real subscription quota has no safe source.

## Review notes (rev, 2026-10-17)
- Verified: `cargo fmt`, `clippy -D warnings`, 128 unit + 17 hook + 21 loopback + 6 probe tests; tmux server pid unchanged (35350); generic `xcodebuild` clean.
- Chat payload and every host path/config dir go through `shell_quote` / `remote_path_expr`; control characters and over-long config dirs are rejected; host ports are `u16`.
- **Low, fixed:** the inspect command's section markers carry a per-call random tag (`===<tag>:SECTION===`), so a settings file from an untrusted repo cannot forge a section (test `a_settings_file_cannot_forge_section_markers`).
- **Open:** no Swift tests for scheme resolution, custom-scheme JSON, accent `#rrggbb` (no iOS test target yet); covered only by build + manual checks.
- **Tradeoffs:** `mouse on` makes tmux handle the mouse (native text selection of scrollback is replaced by tmux copy-mode); the ×5 scroll speed is fixed, not a setting; a free accent color can have poor contrast (e.g. on yellow); custom schemes live only on the phone (no sync); a deleted custom scheme that a profile still names falls back to the slot/general default.
- Not yet run for these features: `hacker`.
