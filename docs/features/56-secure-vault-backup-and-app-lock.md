# Feature 56: Encrypted Vault, Selective iCloud Sync, Full Backup and App Lock

- **Slug**: `secure-vault-backup-and-app-lock`
- **Roadmap**: M45 (extends M32, spec 43 section 6)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (key derivation, authenticated encryption, private keys in iCloud, import of untrusted files, biometric gate)
- **Status**: Design agreed with the owner 2026-10-24. **iCloud sync part implemented 2026-10-24** (vault crypto in the core, signed tier A, sealed tier B for private keys and secret environment values, sync page, migration). Not yet done: backup export/import (section 5), app lock (section 6), vault entries for hook commands, pinned fingerprints, quick commands and appearance; device run.
- **Owner decisions (2026-10-24)**: iCloud sync is **off by default**, the user decides; the passphrase vault is built in the first release of this feature; private keys may go to iCloud and into backups, decided per key; the vault passphrase is typed by hand twice (no generator); the unlock delay survives a relaunch (kept in the defaults); categories are chosen per category, and only environment secrets and private keys are chosen per item; Face ID protects entry, export, showing and syncing keys; existing installs get iCloud sync switched off with one notice.

---

## 1. Purpose

1. Let the user decide what leaves the phone for iCloud, and make what leaves it safe even if the iCloud account or the iCloud Keychain is compromised.
2. Give one complete export/import of the app's data (settings, servers, profiles, snippets, appearance, optionally private keys), using the same cryptography as the vault.
3. Lock the app behind Face ID (with the device passcode as fallback) when the user asks for it.

Non-goals: syncing live sessions, Secure Enclave or FIDO2 keys (spec 43 sections 2.3 and Secure Enclave stay "not ready"), a recovery service (there is none by design), Android.

## 2. Storage tiers

| Tier | Content | Where | Protection |
|---|---|---|---|
| A: metadata | server name, address, port, user; launch profiles and snippets without secrets; quick commands; themes and settings | one synchronizable iCloud Keychain item (as in spec 43 section 6) | Apple's end-to-end encryption. With a vault, every entry is also signed (section 4.4). |
| B: vault | private SSH keys (chosen per key), secret environment values (chosen per item), hook commands (custom / extra), pinned host fingerprints | an encrypted container in a second synchronizable Keychain item | the user's passphrase (section 3); zero knowledge towards iCloud and towards anyone holding the iCloud account |

Without a vault, tier A works as today (opt-in, off by default) and no secret ever syncs. With the vault off, the secret categories are not offered.

## 3. The vault

### 3.1 Keys and container

- `vault_key`: 32 random bytes from the OS CSPRNG, created on the first device that turns the vault on.
- `wrap_key = Argon2id(passphrase, salt, m, t, p)`; `salt` is 16 random bytes; parameters are stored in the header. Defaults: `m = 64 MiB`, `t = 3`, `p = 1`, tuned to about one second on the oldest supported iPhone, never below `m = 32 MiB`, `t = 2` (the importer rejects weaker parameters in a file, section 5.3).
- Domain separation with HKDF-SHA-256, one `info` label per purpose: `ssh-app/vault/wrap`, `ssh-app/vault/entry-enc`, `ssh-app/vault/entry-sig`, `ssh-app/backup/enc`. A key derived for one purpose is never used for another.
- Container: `header || nonce || ChaCha20-Poly1305(wrap_key, nonce, vault_key, aad = header)`. The header is `magic "SSHV" | version u8 | kdf id u8 | m u32 | t u32 | p u8 | salt[16] | created_at u64`. Any header change breaks the tag.
- Entries are encrypted with a key derived from `vault_key` and a fresh 12-byte random nonce each (never a counter that could repeat across devices), `aad = label || entry id || edit time`.

### 3.2 Passphrase

- Typed by hand, twice, with the second field compared in constant time before anything is derived.
- At least 12 characters; weaker phrases get a warning with an estimate (length and character classes, no network lookup) but the minimum is enforced.
- There is no generator and no recovery. Losing the passphrase loses the vault; local data on each device is untouched and can be re-synced after a new vault is made (the screen says this before the first confirmation).
- The passphrase is held as `Data` and zeroed after the KDF; the text fields' own buffers cannot be zeroed (documented, as in spec 43 section 2.1).

### 3.3 Lifecycle

| Action | Effect |
|---|---|
| Create vault | generate `vault_key`, ask the passphrase twice, write the container, store `vault_key` in the local Keychain (`AfterFirstUnlockThisDeviceOnly`, not synchronizable) |
| Join from another device | read the container, ask the passphrase once, unwrap, store `vault_key` locally; the phrase is not kept |
| Change passphrase | re-wrap the same `vault_key` with a new salt; entries stay valid |
| Reset vault | new `vault_key`, all vault entries re-encrypted, the old container and entries deleted from iCloud; other devices see a generation change, drop their copy of the old key and must join again. This is the revocation for a lost device |
| Turn vault off | delete the container and all tier B entries from iCloud; local data stays |
| Wrong phrase | the AEAD tag fails; the screen says "wrong passphrase or damaged container" (no distinction). Attempts are delayed locally (1 s, doubling to 60 s) so a stolen unlocked phone cannot be used as a guessing oracle; offline guessing against a copied container is limited only by Argon2id and the passphrase |

### 3.4 Two devices creating a vault at once

If two devices create a container before seeing each other's, the one with the earlier `created_at` wins (ties: the lexicographically smaller salt). The other device finds its container replaced, drops its `vault_key`, tells the user, and offers to join with the winner's passphrase; its local data is untouched and re-syncs afterwards.

### 3.5 Devices list

Each device writes `{device id (random), name, last sync}` into a vault entry. The list is informational (to spot an unknown device and then reset the vault); it is not an access control.

## 4. Selective sync

### 4.1 What the user chooses

1. A master switch "iCloud sync", default **off**. While off, nothing is read from or written to iCloud.
2. Categories (each its own switch): servers, launch profiles, snippets, quick commands, appearance and settings.
3. Vault categories (shown only with a vault): pinned host fingerprints, hook commands (custom and extra), secret environment values, private keys.
4. Per item, **only** for secret environment values and private keys: a "Sync this" switch on each item. New items of these two kinds default to off regardless of the category switch.
5. "What will be sent": a list of categories and, for tier B, items, with sizes.
6. "Remove from iCloud": deletes everything this app put there (items and tombstones); local data stays. Turning an item or category off removes its remote copy the same way; other devices keep their local copy and do not delete it.

### 4.2 Secret environment values and private keys

- An environment value is a secret when its key name contains `KEY`, `TOKEN`, `SECRET`, `PASSWORD`, `PASSWD`, `CREDENTIAL` or `AUTH` (the list of spec 43). Such values never appear in tier A; with the vault and the per-item switch they go to tier B as `{profile id, name, value}`.
- A private key goes to tier B only if its own switch is on, the vault exists, and Face ID (or the passcode when Face ID is unavailable) succeeded for this action. The screen states: "this key will leave this phone; anyone who knows your vault passphrase and has your iCloud account can use it". Turning the switch off deletes the remote copy; it cannot un-share a key another device already imported (the screen says so).
- A key that arrives from the vault is stored in the local Keychain as `ThisDeviceOnly`, not synchronizable, like any imported key.

### 4.3 Merge rules

Unchanged from spec 43: per id, last writer wins by `updated_at_ms`, ties go to a deletion, then the larger payload; tombstones live 30 days; ids `[A-Za-z0-9_-]` up to 64; payload up to 16 KB; up to 1000 entries. Replay of an older copy is already covered by the edit time: an older entry never replaces a newer one, so no separate `version` counter exists. For tier B the sealed entry authenticates its id and edit time; for tier A the MAC covers id, edit time, deletion flag and payload.

### 4.4 Authenticity of tier A

With a vault, each tier A entry carries `HMAC-SHA-256(entry-sig key, id || version || updated_at || payload)`. An entry without a valid MAC is ignored and reported ("an entry was not signed by your vault"). This closes the risk recorded on 2026-10-23: someone with the iCloud Keychain could previously inject a launch profile or snippet. Without a vault the residual risk stays and is documented.

### 4.5 Sizes and metadata

Tier B payloads are padded to the next multiple of 1 KB so item sizes do not reveal key lengths; the number of entries is the number of enabled items (not hidden; documented).

### 4.6 iCloud state

Signing out of iCloud or turning iCloud Keychain off pauses sync with an explanation in Settings; nothing local is deleted. If the Keychain write fails with a quota or availability error the screen shows it and keeps the local state.

### 4.7 Migration

On first launch after the update: the master switch is **off** for everyone (including users whose sync was on), a one-time notice points at Settings, nothing is deleted from iCloud automatically, and the old per-category flags are ignored until the master switch is turned on.

## 5. Backup file (export and import)

### 5.1 Contents

The user ticks groups: app settings (`AppConfig`), appearance, servers (optionally with pinned fingerprints), launch profiles, snippets and quick commands, hook configuration, and private keys (each key ticked individually; off by default). Anything marked secret (private keys, secret environment values, hook commands) forces encryption of the whole file.

### 5.2 Format

`SSHBK` JSON envelope: `{schema: 1, created, app_version, groups: {...}}`. A file with no secret groups may be written as plain JSON (checksum only, as the existing settings export). A file with secrets is encrypted with the vault container of section 3.1 (`ssh-app/backup/enc`), the passphrase typed twice for the export; it may be the vault passphrase or another. The format is shared Rust code with the vault (`vault_crypto`), so there is one KDF, one AEAD and one set of tests.

### 5.3 Import

- Size limit 4 MB, parsed with a depth limit; unknown schema versions are refused.
- KDF parameters are checked before running: `m` and `t` must be within `[floor, ceiling]` (a hostile file must not make the phone allocate gigabytes or run for minutes).
- A preview lists what is inside and what would change; per group the user picks Merge or Replace; conflicts are shown by name.
- Every record goes through the same validators as an incoming iCloud entry (lengths, control characters, environment names, binary not starting with `-`, id equals entry id). Imported profiles with a custom or extra hook command need an explicit confirmation showing the command.
- Private keys are imported as new keys; an existing key with the same fingerprint is not overwritten. The file is never kept after import.

## 6. App lock (Face ID)

- Settings -> Security: "Require Face ID" (off by default); "Lock after" (immediately, 1, 5, 15 minutes); "Hide in app switcher" (on by default when the lock is on).
- Policy `deviceOwnerAuthentication` (Face ID, then the device passcode), so a broken Face ID cannot lock the owner out. `NSFaceIDUsageDescription` is added to the build settings.
- When the app goes to the background the time is recorded; on return, if the interval passed (or the app was relaunched), a lock screen covers everything, including open sheets, until authentication succeeds. Content is hidden in the app switcher snapshot.
- The SSH socket lifecycle does not change (closed in background, reconnected on foreground, spec "Zero Battery Drain"); the reconnect may run under the lock screen, the terminal is not visible until unlock.
- Extra checks, always on when the lock is on: exporting a backup with keys, showing a private key, turning on key sync, creating or joining a vault.
- Not used: Keychain access control with `biometryCurrentSet` for the SSH key (it would break background reconnect and push actions).
- Notifications stay as they are (neutral text, spec 35); the Approve/Deny actions work from the notification because the approval is already confirmed by the host. The Files extension and the Watch are not covered by the lock (documented; they cannot see private keys).

## 7. Interface (Rust, UniFFI)

```rust
pub struct VaultHeader { pub version: u8, pub m_kib: u32, pub t: u32, pub p: u8 }
pub struct VaultContainer(pub Vec<u8>);

pub fn vault_create(passphrase: Vec<u8>) -> Result<VaultCreated, SshCoreError>; // container + vault_key
pub fn vault_unlock(container: Vec<u8>, passphrase: Vec<u8>) -> Result<Vec<u8>, SshCoreError>; // vault_key
pub fn vault_rewrap(container: Vec<u8>, old: Vec<u8>, new: Vec<u8>) -> Result<Vec<u8>, SshCoreError>;
pub fn vault_seal_entry(vault_key: Vec<u8>, id: String, updated_at_ms: u64, payload: Vec<u8>) -> Result<Vec<u8>, SshCoreError>;
pub fn vault_open_entry(vault_key: Vec<u8>, id: String, sealed: Vec<u8>) -> Result<VaultOpened, SshCoreError>;
pub fn entry_sign(vault_key: Vec<u8>, entry: VaultEntry) -> Vec<u8>;
pub fn entry_verify(vault_key: Vec<u8>, entry: VaultEntry, mac: Vec<u8>) -> bool; // constant time
pub fn backup_export(groups: BackupGroups, passphrase: Option<Vec<u8>>) -> Result<Vec<u8>, SshCoreError>;
pub fn backup_inspect(file: Vec<u8>, passphrase: Option<Vec<u8>>) -> Result<BackupPreview, SshCoreError>;
pub fn passphrase_strength(passphrase: Vec<u8>) -> PassphraseCheck; // ok / warnings, minimum enforced
```

`merge_vault` gains the per-id `version` rule of section 4.3. Errors are typed (`WrongPassphrase`, `Damaged`, `UnsupportedVersion`, `KdfTooWeak`, `KdfTooStrong`, `TooLarge`); messages never include key or passphrase material. Crates: `argon2`, `chacha20poly1305`, `hkdf`, `hmac` (RustCrypto, audited), `subtle` for constant-time comparison, `zeroize` on all key buffers.

Swift (`VaultSync`, `BackupService`, `AppLock`): reads and writes the Keychain items, decides item sets from the switches, calls the core for everything cryptographic, runs `LAContext` for the lock. No cryptography in Swift.

## 8. Threats and mitigations (input for the `hacker` pass)

| Attacker | Capability | Defence |
|---|---|---|
| Holder of the iCloud account and Keychain | read and write tier A and the vault container | tier B needs the passphrase (Argon2id); tier A entries are signed, an unsigned one is ignored; no secret in tier A |
| Same, offline | copies the container and guesses | Argon2id cost, 12-character minimum, warning on weak phrases; documented residual risk for weak phrases |
| Same, replay | restores an older entry or the old container | per-id `version`; container header `created_at` and generation checked |
| Malicious backup file | oversized, deep, hostile KDF parameters, forged ids | size and depth limits, parameter bounds before the KDF, validators, preview, confirmation for commands |
| Thief with the unlocked phone | exports keys, shows a key, joins a vault | Face ID on each of these; delay on wrong passphrases |
| Thief with the locked phone | reads the app | app lock, hidden snapshot |
| Shoulder surfer | sees the passphrase | secure text fields; no clipboard copy of keys without a warning |
| Lost device | still holds `vault_key` | Reset vault rotates the key and invalidates the old copy |

Ruled out: network protocol attacks (no new network protocol), downgrade (versioned, floors on KDF parameters, unknown versions refused).

## 9. Tests

- Rust unit tests: round trip of container and entries; wrong passphrase; tampered header, nonce, tag, aad; nonce uniqueness over many seals; weak and oversized KDF parameters refused; HKDF labels give different keys; MAC verify constant-time path; merge with stale `version`; padding sizes; backup with and without secrets; truncated, deeply nested, oversized files; zeroization of key buffers compiles in.
- Swift tests (simulator): category and item switches decide the entry set; secret env values never reach tier A; a key is excluded unless its switch is on; migration turns the master switch off; lock timing (injected clock); import validators.
- Manual on device (later, on the owner's command): Face ID, two devices joining one vault, iCloud sign-out.

## 10. Order of work

1. Spec review (`rev`), update spec 43 section 3/6 and spec 46 section 3.
2. `vault_crypto` and `backup` in the core, tests, `rev`, `hacker`.
3. Bridge bindings (`swift-bridge-dev`), `rev`.
4. UI: Sync & backup page, Security page, per-item switches, lock screen, migration; `rev`, `hacker`.
5. Merge; device checks on the owner's command.

## Revision notes (`rev`, 2026-10-24)

- Added section 3.4: concurrent vault creation had no rule.
- Controversial, kept as a named tradeoff: tier A is signed only when a vault exists, so a user without a vault keeps the injection risk recorded on 2026-10-23. A mandatory vault for any sync would remove it but would make plain server sync heavy; the screen states the difference when sync is turned on without a vault.
- Controversial, kept: private keys in iCloud are allowed by the owner. The per-key switch, Face ID, vault passphrase floor and the "cannot un-share" warning are the mitigations; a per-device key remains the safer default and the screen says so.

## Implementation notes (2026-10-24)

- Categories (servers, launch profiles, snippets) default to **on** once the master switch is on; the master switch, fingerprints, private keys and secret values default to **off**. The turn-on dialog lists what leaves the phone.
- Deleting or switching off a tier B item removes only its copy in iCloud; nothing in tier B ever deletes data on another device. A device that still has the item switched on uploads it again; the sync page says so.
- Private keys and secret values go through the vault only; the Face ID (or passcode) check runs before a key's switch is turned on and before the vault is created, reset or its passphrase changed.
- Open: hook commands, pinned fingerprints, quick commands and appearance as vault categories; the Face ID app lock; backup export/import; Argon2id cost tuning on the device.
