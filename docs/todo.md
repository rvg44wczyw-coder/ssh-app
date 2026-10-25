# TODO: what the roadmap leaves open

Everything not finished to the end in `docs/roadmap.md` lives here. Review it at the start of a work session and after each chain: take the top of **Next**, close items by deleting them (the roadmap and the findings docs keep the history), add new leftovers as they appear. Last sorted: 2026-10-24.

## Next (can be done without the owner)

Nothing is left here that needs neither the phone nor the owner. Dark, light and the largest accessibility text size were checked in the simulator for Security, Backup, iCloud sync and Hooks (2026-10-24).

## Waits for the phone (device run pending)

Install with `make iphone-full` only on the owner's word. Check on the device:

- [ ] M45: Argon2id cost measured on the phone; quick commands and accent colour in the iCloud sync (two devices).
- [ ] M45 backup (export, encrypted file, import with Skip / Add new / Replace same) and the Face ID app lock.
- [ ] M46 hooks screen; M44 feed (Claude, agy, subagents); delivery marks and queue actions; file links; pinch zoom; light bar.
- [ ] M21, M27, M30 (PNG/SVG share), M33, M34, M37, M38, M39, M41, M42, M43 (see their roadmap rows).
- [ ] Reduce Transparency on the glass screens (the simulator cannot switch it).
- [ ] M43 server's own connection: open Containers/Processes/Ports/Monitor on a server with no tab, background close, return to the app.
- [ ] Push relay T4 on the iPhone.
- [ ] Files extension, Watch registration and Developer Mode.
- [ ] M33: one continuous onboarding wizard and colour blocks for the host fingerprint (the spec says after a device run).
- [ ] M24 watch: the app already shows "iPhone not reachable", never approves without the phone and plays haptics; open is only the run on a paired watch.
- [ ] Files extension: does the Face ID sheet appear inside the extension (`docs/security-findings/2026-10-24-files-face-id.md`, item 5; if not, fall back to the in-app unlock window), favourites, Linux `stat` run.
- [ ] M44 phase 4b: splice only the new screen lines into the feed (a performance change, measure with the in-app timing label before and after).

## Waits for live agents

- [ ] agy dialog tabs (needs a recorded dialog); Ollama multimodal uploads; host usage index; agy: run the "Auto with blocked commands" template live; agy database feed run (owner's go-ahead on permissions).
- [ ] Live hook checks: Codex, Grok, Qwen, Crush, OpenCode, `ollama launch`.

### Not captured yet (found in the Codex/Grok/background-task studies, 2026-10-25)

Mark an item done only after the check has run and the owner has confirmed it. One live run = one permission for one task; the agent's folder is the app's own directory, the tmux socket a private one.

- [ ] Claude Code with two or more background shells: the list layout (spec 59, §10).
- [ ] Claude Code: does `/tasks` exist in this version, or is the footer pill the only way (spec 59, §10).
- [ ] Antigravity: a finished background task, several tasks, and whether its database feed carries them (spec 59, §10).
- [ ] Codex: the queue of messages typed while it works; its approval screens; `/permissions`, `/model`, `/status` and the `/` command list; where it keeps hook trust (`docs/architecture/codex-grok-study-2026-10-25.md`).
- [ ] Grok: the background task list screen (needs the phone approval for the hook, or a hooks-free run); the format of `updates.jsonl` and `chat_history.jsonl`; where it keeps hook trust; the `Ctrl+x` shortcut groups opened one by one.
- [ ] Grok: `/hooks`, `/mcps`, `/usage`, `/dashboard` screens (only `/settings`, `/model`, `/session-info`, `/` and `Ctrl+x` were seen).
- [ ] Spec 11 hooks section: correct the Grok line (it blocks on `UserPromptSubmit`, `PreToolUse`, `Stop`, `SubagentStop`; folder trust gate) and the Codex trust review after the installer writes `hooks.json`.
- [ ] Spec 60 phases 5 and 6 live checks (Codex, Grok) before their adapters are called done.

## Waits for the owner's decision

- [ ] Push relay: the shared secret must become a per-install App Attest before any distributed build.
- [ ] M32: FIDO2 / YubiKey spike (needs a real key), Secure Enclave P-256 key.
- [ ] M47: macOS app: spec `docs/features/58-macos-app.md` is ready; waits for the owner's decisions D1-D5 (Catalyst, PTY in the core, direct distribution, "This Mac" entry, Sparkle) and Intel yes/no. Phase A (Catalyst, remote half) can start on the word.
- [ ] M28 Live Activities and M31 iPad: specs reviewed, implementation deferred by the owner.
- [ ] M26 / M40 (`feature/mesh`, local branch): deferred.
- [ ] Native Windows host (spec 34, W1–W7), Android stays excluded.
- [ ] Payments (M20): switched off; enabling needs the owner's word.
- [ ] Apple `container` runtime: waits for a recorded sample.
