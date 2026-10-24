# Backup file and app lock: adversarial pass (2026-10-24)

## Scope

`crates/core/src/backup.rs`, `ios/Sources/SSHApp/Services/{BackupService,BackupController,AppLock}.swift`, `Views/{SettingsBackupPage,SettingsSecurityPage}.swift`, manager `restore` methods. Local only: Rust unit tests and Swift tests in the simulator; no real tmux, no network.

Categories: InputValidation, DoS, KeyConfusion, MetadataLeak applied; MITM, Replay, Downgrade have no surface (a file, no protocol; the container version is fixed and a plain file cannot carry secrets); Timing is not exploitable (an offline file, Argon2id dominates).

Live tests run: 10 core tests (the tenth flips every byte and truncates every length of a plain and an encrypted file plus junk inputs: no panic, no hang) (forged secret flag in a plain file, tampered checksum, tampered ciphertext, wrong passphrase, oversized file, deep nesting, KDF cost out of bounds, bad group id, round trip); 3 Swift tests on hostile records (bad UUID, option-looking host, malformed JSON) and plain-file refusal; `shouldLock` timing tests.

## Findings

1. **InputValidation, Low** (`BackupController.load`): `Dictionary(uniqueKeysWithValues:)` over preview rows would trap on a repeated group id. Not reachable today (core groups are a JSON object, keys are unique) but one parser change away from a crash on a hostile file. Fixed: `uniquingKeysWith`.
2. **MetadataLeak, Low** (`BackupController.export`): a plain backup (server names, hosts) stayed in the temporary folder after sharing. Fixed: exports are removed before the next export and when the page closes.
3. **Controversial, kept as a named tradeoff**: imported launch profiles carry a binary and arguments the host later runs. Same trust as the existing vault sync (fields are validated, nothing runs at import, the owner starts a profile). Hook commands, which run on their own, are in a separate secret group, shown in full and need an explicit confirmation.
4. **Controversial, kept**: the lock is a UI gate, not encryption of data at rest; secrets stay protected by Keychain `AfterFirstUnlockThisDeviceOnly`. The screen says "locks the app", not "encrypts".

## Verdict

Findings (highest Low); both fixed. Hostile files do not crash, overwrite keys or run commands.
