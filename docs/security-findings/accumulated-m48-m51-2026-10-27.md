# Review and security pass over the accumulated work (2026-10-27)

## 1. Scope

Reviewed: the uncommitted tree on top of `08dfdaa` plus the key-in-core refactor committed in `08dfdaa`:
`key_store.rs` (generate, import, seal, stable seal, export, guarded copies), the M50 hooks additions
(`agent_hooks/config.rs`, `host_api/hooks.rs`, the in-feed offers), the M51 sandbox parts
(`crates/host/src/egress.rs`, `host_api/sandbox.rs` `sandbox_blocked_hosts_for`, `sandbox.rs`
`review_files_command`, the `blocked` category in Worker and `push.rs`, the Notification Service
Extension, `PushNames`, `SandboxRegistry`) and the agent-registry fields.

Method: code reading plus the project's own loopback tests (first contact, extension lookup in one exec,
exclude against a real git repository) and new tests written for each finding. Live attacks only against
throw-away local data (tempdirs, a local `sh` flooding pipes); nothing else was touched. The tmux server
pid was 35350 before and after.

Not covered: **M48 (background tasks) has no implementation yet** (spec 59 only; the Claude "Shell details"
fixture is parsed by the existing screen code), so there was nothing to attack. Coverage percentages were
not measured. `vault_crypto.rs` / `backup.rs` / `vault_sync.rs` were audited on 2026-10-22 and 2026-10-24
and are unchanged since; only re-read for domain separation (labels are distinct, entry AAD binds id and time).

Ruled out: replay/downgrade (no new negotiation; the old `SSHK2` blob is still read, which is a deliberate
migration path, see 1), weak randomness (all new randomness comes from `rand::rng()`; the stable seal is
deterministic on purpose).

## 2. Findings (all fixed the same day)

1. **KeyConfusion / nonce reuse, Low** - `key_seal_stable` (`key_store.rs`): the nonce was derived from the
   secret and the key's fingerprint only. The same key stored again with another text (re-import, other
   comment or check value) sealed two different plaintexts under one key and nonce (ChaCha20-Poly1305
   nonce reuse: leaks the XOR of the two plaintexts and allows tag forgery). Fixed: new form `SSHK3` whose
   nonce also depends on a SHA-256 of the key text, and which carries the nonce in the blob; `SSHK2` blobs
   are still opened, never written. Test: `two_texts_of_one_key_never_share_a_nonce_and_old_blobs_still_open`.
2. **Weak secret floor, Low** - `key_seal` accepted an 8-byte password for a file that can be attacked
   offline for as long as it exists. The app already asks for 12; the core now enforces 12.
3. **PathTraversal, Low** - `exclude_hooks_command`: in a hostile repository `.git/info/exclude` (or the
   `.git/info` folder) can be a symlink, so the appended line landed in its target. Fixed: the command
   refuses when either is a symlink. Live test on real git: `a_symlinked_exclude_file_or_info_folder_is_refused_and_nothing_is_written_through_it`.
4. **DoS, Medium** - `egress.rs` `EngineLogs::recent`: stdout and stderr were read one after the other with
   a cap, then `wait()` with no timeout. A noisy proxy log (the agent controls what the proxy logs) could
   fill the stderr pipe while the reader waited on stdout, or stop the reader at the cap while the engine
   blocked writing; the watcher thread then hung and its `busy` flag stayed set, so no further `blocked`
   push ever went out. Fixed: both pipes are drained on their own threads (first 1 MB kept, the rest
   discarded) and the engine is killed after 10 s. Test: `a_noisy_process_cannot_block_on_a_full_pipe` (8 MB on each stream).
5. **DoS, Medium** - `egress.rs`: every newly refused name (up to 100 per sandbox) produced a push, and the
   relay allows 20 pushes an hour per install, shared with approval pushes. An agent inside a sandbox could
   invent hosts and use up the budget so that approval pushes were refused. Fixed: one `blocked` push per
   sandbox per 5 minutes and 4 per hour in all; suppressed refusals still show in the phone's Blocked
   list. Test: `refusals_cannot_use_up_the_push_budget`.
6. **Logic (isolation), Low** - `SandboxRegistry.records(on:in:)` showed the default server's sandboxes
   (`serverId == nil`) on every server's screen, so Remove/Allow could be sent to the wrong server. Fixed:
   strict match. Test extended in `SandboxRegistryTests`.
7. **MetadataLeak, none found** - the `blocked` push carries the fixed text and the `sid`; the host name is
   fetched by the extension over its own pinned SSH and never passes the relay. Host names are validated
   DNS names (no wildcards, no IP literals) before they reach a banner.
8. **Secrets as strings, none found** - no `#[uniffi::export]` item takes or returns a password, passphrase,
   token or private key as `String`; `derive_public_key(String)` and `parse_private_key(&str)` are Rust-only.
   The export path needs the three acknowledgements; keys in the Keychain are `AfterFirstUnlockThisDeviceOnly`,
   non-synchronizable; guarded copies need user presence.

## 3. Controversial points

They are in `docs/decisions.md` (open until the owner decides): the extension holding the SSH key, banner
text on the lock screen, the shared relay secret, and the default-server gap in the extension lookup.

## 4. Verdict

Findings (highest severity: Medium), all fixed. `cargo clippy --workspace --all-targets -D warnings` clean,
`ssh-core` 598 and `ssh-app-host` 121 unit tests plus integration tests pass, Swift 209 tests pass.
