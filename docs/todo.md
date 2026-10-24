# TODO: what the roadmap leaves open

Everything not finished to the end in `docs/roadmap.md` lives here. Review it at the start of a work session and after each chain: take the top of **Next**, close items by deleting them (the roadmap and the findings docs keep the history), add new leftovers as they appear. Last sorted: 2026-10-24.

## Next (can be done without the owner)

- [ ] M45: more vault categories: pinned fingerprints, quick commands, appearance. Hook commands stay out of the iCloud sync (code that runs on the host); they travel only in the backup file.
- [ ] M45: Argon2id cost tuning measured on the phone.
- [ ] M44: tail splice of the screen into the feed.
- [ ] M43: the server's own connection for the server-level screens (today they use a connected session).
- [ ] M27: Ollama multimodal uploads.
- [ ] M33: one continuous onboarding wizard, colour blocks for the fingerprint.
- [ ] Files extension extras: Face ID per server, favourites; Linux `stat` run.
- [ ] Glass pass: check dark / light / Reduce Transparency / large text on every new screen.
- [ ] File links in the subagent feed (only the main feed has them).
- [ ] agy dialog tabs.
- [ ] Host service P2: `tmux -C` and the usage index (see roadmap, host-side service).
- [ ] M24 watch: "open iPhone" state when the phone is unreachable (never auto-approve), haptics, watchOS target in `project.pbxproj`.

## Waits for the phone (device run pending)

Install with `make iphone-full` only on the owner's word. Check on the device:

- [ ] M45 backup (export, encrypted file, import with Skip / Add new / Replace same) and the Face ID app lock.
- [ ] M46 hooks screen; M44 feed (Claude, agy, subagents); delivery marks and queue actions; file links; pinch zoom; light bar.
- [ ] M21, M27, M30 (PNG/SVG share), M33, M34, M37, M38, M39, M41, M42, M43 (see their roadmap rows).
- [ ] Push relay T4 on the iPhone.
- [ ] Files extension, Watch registration and Developer Mode.

## Waits for live agents

- [ ] agy: run the "Auto with blocked commands" template live; agy database feed run (owner's go-ahead on permissions).
- [ ] Live hook checks: Codex, Grok, Qwen, Crush, OpenCode, `ollama launch`.

## Waits for the owner's decision

- [ ] Push relay: the shared secret must become a per-install App Attest before any distributed build.
- [ ] M32: FIDO2 / YubiKey spike (needs a real key), Secure Enclave P-256 key.
- [ ] M47: macOS app (no spec yet; `doc-dev` first; sandbox question).
- [ ] M28 Live Activities and M31 iPad: specs reviewed, implementation deferred by the owner.
- [ ] M26 / M40 (`feature/mesh`, local branch): deferred.
- [ ] Native Windows host (spec 34, W1–W7), Android stays excluded.
- [ ] Payments (M20): switched off; enabling needs the owner's word.
- [ ] Apple `container` runtime: waits for a recorded sample.
