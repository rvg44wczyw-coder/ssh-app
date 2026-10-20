# Changelog

## Unreleased (on top of `0620c38`)

### Agent feed
- Feed is parsed from the tmux screen (`crates/core/src/screen.rs`) and merged with the agent's session file: folded groups ("Searched for N patterns", "Ran N commands") open with their calls and results, Edit/Write show the full diff, Ctrl+O detailed transcripts are understood.
- Question card: multi-question forms with a tab strip, Back/Next, review tab, option descriptions; extra-keys row; fixed overlapping tool frames.
- Claude tabs start with `--verbose`; Antigravity uses the file merge.
- Attachments: paperclip menu (photos up to 6, files), choice to keep on the host (`~/.ssh-app/inbox`) or temporarily.

### Sessions
- Restart button (Theme / Rename / Restart / Close): new conversation or continue last, command rebuilt from the profile so new hooks apply.
- Status symbol on session cards and tabs (finished, needs you, question, approval) replaces the banner strip.
- Fix: tab no longer dies after the agent exits (`+m` in the interactive wrapper).
- Host OS misdetection (WSL card on a Mac) fixed.

### Menus
- Quick menu and session menu: configurable order, hidden items, columns (2–6) and icon size in Settings → Appearance; panel and bottom button row follow the width.

### Push
- Tap on a push opens the right session (opaque `sid`); pushes are suppressed while the app is in the foreground.
- Fix: push tap and status symbol for agents without a SessionStart hook (Antigravity): any first hook event registers the session's tmux name; the app waits for the name and for restored tabs.
- Antigravity questions are detected on the host from the tmux screen and sent as `waiting` pushes (`crates/host/src/screenwatch.rs`).
- Default relay URL in the app; `make relay-config` injects the secret into a git-ignored file for personal builds only. TODO before distribution: per-install App Attest instead of the shared secret (`docs/features/35-push-relay.md` §5).

### Docs
- `docs/features/31-agent-feed.md` (§9 screen feed, §10 attachments), `docs/features/35-push-relay.md` (§4.1, §4.2, §5, agent questions).
