# Open decisions

Questions that came out of reviews and are the owner's to decide. An item stays here until the owner
decides; then it moves into the spec it concerns and is deleted from this file.

Decided 2026-10-27 and moved into the code and specs: D1 (option B: per-server switch "Name blocked hosts in banners", `ServerProfile.bannerLookup` -> `SharedServer.bannerLookup`, checked by the extension), D2 (kept; the Push card in Settings says the names show on the lock screen and how to hide them), D4 (a sandbox without a server id is published with the default server's id). D3 is planned in `docs/features/35-push-relay.md` ("Per-install App Attest") and waits for the owner's word to build and to deploy the Worker.

## D3. Shared relay secret (also in `docs/todo.md`)

Plan written, nothing deployed. Needs the owner's OK for: the App Attest entitlement on the app id, the Worker redeploy, and the choice TypeScript or `workers-rs`.
