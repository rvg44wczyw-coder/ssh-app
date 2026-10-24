# Encrypted vault and selective iCloud sync (spec 56): review and attack pass

Date: 2026-10-24. Scope: `crates/core/src/vault_crypto.rs`, `vault_sync.rs` (id check), `ios/Sources/SSHApp/Services/{EncryptedVault,VaultSecrets,VaultSync,Biometrics}.swift`, `Views/SyncSettingsView.swift`. Method: code analysis plus the unit tests below (all local, in-memory stores; nothing touched the real Keychain, iCloud, tmux or Docker; tmux pid 35350 before and after).

## Applied / ruled out
Applied: key derivation, authenticated encryption, replay, downgrade, key confusion, timing, randomness, input validation, DoS, metadata leakage. Ruled out: network protocol (no new protocol), sandbox/subprocess (none spawned), path traversal (no paths).

## Findings (all fixed before this note)
1. **Downgrade, Medium.** With the vault state `off` (container deleted in iCloud) the app used no key, so tier A entries were accepted unsigned: an attacker who can delete the container could then inject a profile or snippet. Fix: state `removed` (a device that holds a key but sees no container) pauses sync. Test: `testADeletedContainerPausesSyncInsteadOfDroppingTheChecks`.
2. **Brute force, Low.** The unlock delay lived in memory, so relaunching the app reset it. Fix: failures and block time kept in the defaults. Test: `testTheUnlockDelaySurvivesARelaunch`.
3. **Vault replacement, Low (DoS / confusion).** A replaced container silently turned into "enter the passphrase". Fix: `replaced` state with an explicit message; the old key is dropped and the real passphrase does not open the attacker's container. Test: `testAReplacedContainerIsReportedAndTheOldKeyDropped`.

## Verified by tests
- Every byte of a container (header, nonce, tag) and of a sealed entry is authenticated; an entry cannot be moved under another id or given another time; weak and excessive KDF parameters are refused before the derivation runs (under 200 ms); a backup container does not open as a vault; nonces never repeat over 2000 seals; sealed sizes are padded to 1 KB; purposes use different HKDF keys; the MAC covers every field and rejects other keys and truncated tags.
- Signed list: forged, edited or unsigned entries are dropped and counted; without a vault nothing is checked (documented residual risk).
- Tier B: only enabled items upload; dismissed items are not imported again; turning off removes only the iCloud copy; a switched-off category is left untouched; incoming keys must match their public half; incoming env values are validated (secret-looking name, no control characters, id equals content, known profile).

## Residual risks (documented, accepted)
- Without a vault the shared lists are protected only by iCloud Keychain; anyone holding the iCloud account could inject a profile or snippet (nothing runs without a tap).
- A weak vault passphrase can be guessed offline from a copied container; Argon2id and the 12-character minimum only slow it. The screen warns.
- Key material crossing the FFI boundary is a plain byte array; the core zeroizes its own copies, the Swift side cannot.
- A device that still has an item switched on uploads it again after another device removed it.
- Face ID is checked in the UI before key sharing; it is not enforced inside `VaultSync` (a compromised app process could bypass it, which already means the key is exposed).
- A full-suite run showed one timing-dependent failure in `feed_stream` under CPU load (passes alone, three runs); not related to this change.

Verdict: findings fixed; no open Critical, High or Medium item. The device run (two phones, iCloud sign-out) is still to do.
