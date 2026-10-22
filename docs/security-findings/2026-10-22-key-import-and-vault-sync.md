# Security review: passphrase key import and iCloud server sync (feature 43 / M32 slice)

Date: 2026-10-22. Reviewer: `rev` + `hacker` passes run by the implementing session (no separate
agent); live tests were local unit tests only, nothing touched tmux, sshd or the network.

## Scope

`crates/core/src/keys.rs` (`import_private_key`, `private_key_needs_passphrase`),
`crates/core/src/vault_sync.rs`, `ios/Sources/SSHApp/Services/VaultSync.swift`, the import sheet and
sync section in `SettingsView.swift`, `SSHKeyStore.importKey`.

Applied: key handling, input validation, DoS, metadata leakage, trust transfer (MITM via sync),
replay/rollback of the synced blob. Ruled out: network protocol parsing (no new wire protocol; the
transport is the OS keychain sync), downgrade (no negotiation), timing (no secret comparison added),
subprocess/plugin execution (none).

Live tests actually run: a key file whose bcrypt rounds were patched to `u32::MAX` (refused in well
under a second, before any derivation); wrong, missing and empty passphrase; a non-Ed25519 key;
garbage text; hostile merge entries (empty/odd ids, oversized payload, 1050 entries, stamp in the far
future); hostile synced server payloads (`-oProxyCommand=…` host, whitespace host, port 0, `-l root`
user, non-JSON). Not tested: real iCloud transport between two devices (needs two devices).

## Findings

1. **DoS / InputValidation, Medium, fixed.** A key file chooses its own bcrypt rounds. Fix:
   `MAX_BCRYPT_ROUNDS = 500` checked before deriving; test `a_file_that_asks_for_billions_of_rounds_is_refused_at_once`.
2. **Replay / rollback, Low, fixed.** A synced entry stamped far in the future would win every later
   edit (also what a device with a wrong clock produces). Fix: remote entries more than one day ahead
   are ignored (`MAX_CLOCK_SKEW_MS`); test `an_entry_stamped_in_the_far_future_is_ignored`.
3. **InputValidation, Low, fixed.** A server arriving from iCloud is data from outside the device.
   Fix: `SyncedServer.isSane` refuses a leading `-`, whitespace or control characters, over-long
   values and port 0; the core caps entry count, id charset and payload size. Tests in `VaultSyncTests`.
4. **Trust transfer (MITM), Medium by design, mitigated.** Syncing pinned host fingerprints would let
   whoever can write the vault pin a host key on every device. Mitigation: off by default with a
   footer that says so; with the toggle on, a different fingerprint never replaces a local pin and is
   shown as a conflict. An injected *server* without a fingerprint is only a list entry: the first
   connection still shows its key (trust on first use) and its `keyId` is never synced, so it would
   use the default key, whose signature is bound to that session and reveals nothing.
5. **MetadataLeak, Low, accepted.** Server names, addresses and users sit in an iCloud Keychain item,
   readable by anything that holds the account's keychain (same trust as the account). No keys, no
   key choice. The toggle turns it off.
6. **Memory, Low, accepted (stated in the spec).** The passphrase is `Data` and wiped in Rust, but the
   text a user typed lives in SwiftUI's field buffer and a Swift `String` copy until it is released;
   neither can be zeroized from Swift. The armored key text on the Swift side has the same limit as
   before this change.
7. **Availability, Low, accepted.** Someone with write access to the vault can delete servers by
   writing tombstones; the user re-adds them. Tombstones expire after 30 days.

## Verdict

Findings (highest severity Medium); 1–4 fixed with tests, 5–7 accepted and documented in
`docs/features/43-hardware-keys-and-vault-sync.md` section 6.
