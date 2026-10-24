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

## Waits for the owner's decision

- [ ] Push relay: the shared secret must become a per-install App Attest before any distributed build.
- [ ] M32: FIDO2 / YubiKey spike (needs a real key), Secure Enclave P-256 key.
- [ ] M47: macOS app (no spec yet; `doc-dev` first; sandbox question).
- [ ] M28 Live Activities and M31 iPad: specs reviewed, implementation deferred by the owner.
- [ ] M26 / M40 (`feature/mesh`, local branch): deferred.
- [ ] Native Windows host (spec 34, W1–W7), Android stays excluded.
- [ ] Payments (M20): switched off; enabling needs the owner's word.
- [ ] Apple `container` runtime: waits for a recorded sample.
