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

- [ ] Spec 60 phase 4 follow-up: move the per-agent settings screens (hooks, launch profiles, permissions, skills, themes) onto the registry together with the Codex/Grok hook screens, then add the "no agent id literal in the feed code" guard test.
- [ ] Spec 60 phase 4: device run of the feed for Claude and agy (commands menu, keys, subagents, database feed) after the refactor.

## Waits for the owner's decision

- [ ] Push relay: the shared secret must become a per-install App Attest before any distributed build.
- [ ] M32: FIDO2 / YubiKey spike (needs a real key), Secure Enclave P-256 key.
- [ ] M47: macOS app: spec `docs/features/58-macos-app.md` is ready; waits for the owner's decisions D1-D5 (Catalyst, PTY in the core, direct distribution, "This Mac" entry, Sparkle) and Intel yes/no. Phase A (Catalyst, remote half) can start on the word.
- [ ] M28 Live Activities and M31 iPad: specs reviewed, implementation deferred by the owner.
- [ ] M26 / M40 (`feature/mesh`, local branch): deferred.
- [ ] Native Windows host (spec 34, W1–W7), Android stays excluded.
- [ ] Payments (M20): switched off; enabling needs the owner's word.
- [ ] Apple `container` runtime: waits for a recorded sample.

## Status line, one look for every agent (2026-10-26)
- [x] Host usage probe: Codex and Grok now report model, context in use and window (Codex also its plan limits, read from this session's rollout, not the newest on the host); Antigravity and Claude as before. Git comes first in the line for every agent and is asked again once the tab is connected.
- [ ] Plan limits for Antigravity (`/credits`) and Grok (`/usage`): Grok's `/usage` after the owner logged in with a paid plan (checked live 2026-10-26) shows a "Usage limit" tab: "Weekly limit (SuperGrok Lite)", a bar with `10%` (used) and "Resets: November 2, 11:50" (done 2026-10-26: `crates/host/src/grokquota.rs` reads it into a `7d` chip the same way as `agyquota.rs`; the reset date has no year, so the next such local date is taken; live-checked on the host, the chip on the phone is not); Antigravity's `/usage` ("View model quota usage", checked live 2026-10-26) DOES show plan limits: a "Models & Quota" screen with groups (Gemini Flash/Pro; Claude Opus/Sonnet/GPT-OSS; Gemini 4 Argon), each with "Weekly Limit Remaining NN.NN%" + "Refreshes in 120h 10m" and "Five Hour Limit Remaining NN.NN%" + "Quota available" (done 2026-10-26: `crates/host/src/agyquota.rs` runs agy in a private tmux every 15 min while an agy tab exists, parses the screen, caches `~/.ssh-app/run/agy-quota.json`; live-checked on the host, the chips on the phone are not; the Gemini 4 Argon group had no windows in the capture); Antigravity's `/credits` shows only "Remaining AI Credits: AI Credits not enabled (enable in /settings)" and two actions, no plan percentages; if the owner enables AI credits the line would carry a number: capture it then and add the chip. Antigravity's start banner prints the account e-mail: never record it. Check on the phone that the model, `NN% ctx` and git appear for all four agents (needs a host helper rebuild: `make iphone-full`).
- [ ] The refresh button: owner to choose (hint / remove with git refreshed after each turn / remove).

## Grok (spec 60, phase 6)
- [x] History parser, `GrokSource`, `GrokDialect`, keys (Enter queues, Ctrl+Enter sends now, Up+`e` pulls), command menu, permission prompt — captured live 2026-10-26.
- [x] Answering the permission dialog: checked live, Down moves the `(●)` marker and Enter confirms (also the digit key); the app's arrows + Enter work as they are.
- [x] `/model` and `/effort` pickers are prompts (cards); the plain slash menu is not. `/plan` captured (toast `Plan mode: on`, `· plan` in the composer frame).
- [x] Ctrl+U clears Grok's composer (checked live). `tool_result` `exit: N` (N ≠ 0) marks a failed command.
- [x] `permission_grok-pager.toml`: my accidental `tasks__list` entry removed (list empty again).
- [ ] Not captured: `/resume` modal (lists earlier prompts of the owner, so no fixture), `/rewind` dialog (bulleted entries with no numbers), `/settings` modal, `/tasks` with a running task, background shell (`block_until_ms`), subagents, first-run folder-trust screen, Grok hook-trust gate. They show as folded "Terminal output".
- [ ] Decision for the owner: our PreToolUse hook waits for the phone up to its timeout (120-130 s) by design (spec 11); with no phone connected the Grok command is held that long, then Grok's own flow continues.

## Codex (spec 60, phase 5)
- [x] `CodexDialect` + `ScreenDialect::Codex`, keys, command list, approval prompt, `/ps` `/stop` — captured live 2026-10-26.
- [x] Enter (steers) versus Tab (queues): `AgentKeys.queue_keys` (Tab for Codex); `submit(.normal)` sends it instead of Enter while the agent works.
- [x] `/permissions` and `/model` pickers are prompts with the note split off the label; hook trust: Codex's `/hooks` screen has `t trust all`, the note is shown under the profile's hooks row.
- [x] `/hooks` captured (full-screen table; shows as folded terminal output).
- [ ] Not captured / parsed: `/resume`, `/review`, `/diff`, `/skills`, `/agents` (lists the owner's items), `/config` (tabbed modal); Plan mode (`Plan mode (⇧tab to cycle)` on the status line, `Model changed … for Plan mode`) works as plain text; reasoning-effort picker.
- [ ] Device run of the feed with a real rollout (needs a phone build; the owner has not asked for one).
- [x] Codex plan (checked live 2026-10-26 after the owner re-logged in): rollouts now say `plan_type: go` with ONE window, `window_minutes: 43200` (monthly), `/status` shows "Monthly limit: 100% left (resets … 25 Nov)"; the host already turns it into a `30d` chip. Docs (learn.chatgpt.com/docs/pricing, read 2026-10-26): Plus and Business have a five-hour window ("Weekly limits may also apply"), Pro "currently has no five-hour limit" (weekly only), Free and Go are not described and no monthly limit is mentioned (the live Go account does have a monthly window). So the chip label comes from `window_minutes` of whatever the plan sends (5h, 7d, 30d), nothing is hard-coded per plan; Plus/Pro rollouts still unseen. `/usage` opens a menu "View analytics" / "Redeem reset" (the second one was never opened); the analytics screen holds the account name: never record it.
- [ ] Hooks per project (spec 61, owner's request 2026-10-26): project scope by default, all-sessions hooks only on request; Claude `.claude/settings.local.json` and agy `.agents/hooks.json` first; Codex/Grok project paths need a live check (owner: later, with the phone verification); separate: make the hook return at once when no phone session is attached instead of waiting the full timeout.
- [ ] Hooks are per server (spec 61 §5.0): the screen opens under a server and lists the agents found on that host; `AgentProfile.serverIds` (empty = all servers) so profiles and their folders belong to the servers where they exist.
- [ ] Hooks by category (spec 61 §5.0.1): free-text category on profiles and folders, grouping and bulk enable/remove per category.
- [ ] Host helper `hooks-overview` (spec 61 §5.0.2): finds agents and projects on the host itself (running panes, Claude/Codex/Grok/agy histories) and reports hook state per target; read-only.
- [ ] Hooks screen is built (spec 61 progress table) but never seen on a device: check Settings → Agents → Hooks on the phone after a build: agents found on the server, project rows, Enable/Remove, the all-sessions warning.
- [ ] DeepSeek (owner, 2026-10-26: "не трогай дипсик пока", do nothing on it until the owner returns to it): clarify what is meant (a model via an existing agent such as OpenCode/Qwen/Claude Code with a custom base URL, or a DeepSeek CLI) and add it to the agent registry, launch profiles and the Hooks screen.

- [ ] Host-key confirmation ("Trust this server?") and redacted logs: check on the device after a build.
- [x] Private key no longer crosses UniFFI as a `String` (external review, 2026-10-26; owner's rule now in AGENTS.md). Done: `crates/core/src/key_store.rs` (Keychain read/write in Rust, same service/account names as before, so old keys are read in place; in-memory store under tests and `test-support`), sessions/remote fs/approvals take a `KeyRef` instead of PEM text; generate/import (bytes + passphrase bytes)/delete/publish-guarded-for-Files exported; plain export only with three confirmations (`ExportAcks`, typed `EXPORT`, UI sheet + owner check); backup and vault sync carry keys only sealed by the core (Argon2id + ChaCha20-Poly1305 under the backup passphrase; HKDF + ChaCha20-Poly1305 under the vault key, stable output). Old FFI functions (`derive_public_key`, `import_private_key`, `generate_ssh_keypair`, `sign_approval_*`) are no longer exported. Left: (a) delete `LegacyKeyMigration.swift` and its call in `SSHApp.swift` after the first launch on the phone; (b) the Keychain backend in `key_store.rs` is only compile-checked (the package test host has no Keychain entitlement, -34018, so Swift key tests skip there; core tests use the in-memory store): on the device check generate, read of keys written by the old code, delete, guarded copy, and then migration line in the log, connect, Files extension (guarded copy now asks the system without the 60 s reuse context), key export, backup round trip, two-device vault sync of a key; (c) the pasted-key text field and the passphrase `SecureField` still hold text in Swift until import (UI limit, cleared right after); (d) remove `SyncedKey` JSON from older vault copies is not needed (nobody else has a build).
- [x] Sandbox live check for all four agents (2026-10-26): images build and the agent runs (`claude` 2.1.310, `codex` 0.164.0, `grok` 1.0.70, `agy` 1.4.7), isolation and proxy allow-list hold, review fetch works. Not checked: an agent logging in and working inside the container, Podman, offline profile, auto-approve profile.
- [ ] Sandbox allow-list managed from the phone (add / swipe to remove / Allow on refused hosts) is built and compiles, never seen on a device. Idea from the owner: let the agent's own request update the list (today an agent's refused connection already shows up with Allow; an explicit agent-side "ask for host" request through the pending queue is not built).
- [x] Sandbox login (2026-10-26, owner: "всегда копировать логин"): the agent's login file (Claude `.credentials.json` from the configured folder, Codex `auth.json`, Grok `auth.json`, agy `antigravity-oauth-token`) is copied by `cp` on the host into the sandbox home when absent (toggle in the sandbox screen, default on; never a symlink; owner-only). Checked live for Claude Code: skipped the login screen and answered a prompt from inside the container. Not checked live: Codex, Grok, agy copies, and whether a copy and the host's own token disturb each other when one refreshes. Running in Docker stays optional ("Run without a sandbox").
- [ ] Sandbox: permanent allowed hosts per agent (2026-10-26): kept on the phone (UserDefaults), added to every new sandbox of that agent; “Allow always” on a blocked host. Compiles, never seen on a device; synced through iCloud (Settings → iCloud sync → What to sync → Sandbox allowed hosts, on by default; two-device check pending); also in the backup file (group “Sandbox allowed hosts”, plain, not encrypted; Skip / Add / Replace on import).
- [ ] Sandbox (spec 62, M51), what is left (2026-10-26): changed-file warnings in the review list (`sandbox_review_flags` is exported, no call yet); the "Unprotected" mark on the tab; an install offer when no engine is found; per-server profile binding; an explicit "agent asks for a host" request through the pending queue and a push for a refused host; live checks of the Codex/Grok/agy login copy (and whether a copy and the host token disturb each other when one refreshes), Podman, the offline and auto-approve profiles; the whole sandbox screen on a device.
