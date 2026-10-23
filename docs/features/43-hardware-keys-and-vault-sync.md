# Feature 43: Hardware Security Keys (FIDO2 / YubiKey), Key Import & iCloud Keychain Sync

- **Slug**: `hardware-keys-and-vault-sync`
- **Target Release**: Milestone 32 (Enterprise Security & Cross-Device Key Management)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (cryptographic key import, FIDO2/WebAuthn assertions, iCloud sync encryption, Secure Enclave)
- **Status**: Reviewed and corrected (`rev`, 2026-10-20). **Implemented 2026-10-22: passphrase key import and the iCloud server sync** (section 6). Hardware keys (FIDO2) and the Secure Enclave key stay "not ready": the first needs the spike of section 2.3 with a real YubiKey, the second its own signing path. Neither can be proved without a device.

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
1. **Hardware Token Enforcement**: Many corporate and production environments strictly forbid pure software SSH keys, mandating hardware security tokens (`ssh-ed25519-sk` or `ecdsa-sk` via YubiKey or FIDO2 authenticator).
2. **Key Migration Friction**: Developers already have established SSH keys (`~/.ssh/id_ed25519` or 1Password SSH Agent) on their laptops. Generating an isolated key on the phone and manually pasting it into `authorized_keys` across 10 servers creates high friction.
3. **Multi-Device Sync**: Developers using both an iPhone and an iPad currently have to re-enter host configurations, Tailscale IPs, port rules, and snippets separately on each device.

### 1.2 In Scope
- Support for OpenSSH FIDO2 security keys (`sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com`) via NFC and USB-C authenticators (YubiKey 5 Series). iOS offers no raw CTAP2 over NFC/USB-C and WebAuthn cannot use an `ssh:` relying party, so this needs a named third-party library (for example YubiKit) and our own assembly of the `sk-` signature; that is the first deliverable (2.3).
- Secure private key import:
  - Standard OpenSSH format (`-----BEGIN OPENSSH PRIVATE KEY-----`).
  - Passphrase-encrypted keys decrypted locally in memory. OpenSSH uses bcrypt-pbkdf (not Argon2); a high rounds count takes seconds on a phone, so the UI shows progress and a cancel button. The passphrase travels as `Data`, not `String` (a Swift `String` cannot be zeroized); what stays out of reach is the text field's own buffer, and the spec says so.
  - Apple Secure Enclave key generation option (`kSecAttrTokenIDSecureEnclave`). **Secure Enclave keys are P-256 (`ecdsa-sha2-nistp256`), never Ed25519**: a separate key type with its own signing path in russh.
- End-to-End Encrypted iCloud Keychain Vault Sync:
  - Syncs server profiles, connection metadata, launch profiles, and snippets. Pinned host fingerprints sync **only** if the user turns that on (default off): syncing them transfers trust to the other device, which is an explicit decision.
  - **Zero-Knowledge Principle**: Private keys are **never** synced to iCloud. Private keys remain pinned to the local device's hardware Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`). Only public keys and host metadata are synced.

### 1.3 Out of Scope
- Syncing raw unencrypted private key material across iCloud (strictly prohibited).

---

## 2. Cryptographic Domain Models & Protocols

### 2.1 Rust Core FIDO2 & Key Models (`crates/core/src/crypto_vault.rs`)

```rust
use ssh_key::{Algorithm, PrivateKey, PublicKey};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyType {
    Ed25519,
    Ed25519Sk { application: String, flags: u8 },
    EcdsaP256Sk { application: String, flags: u8 },
    Rsa4096,
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ImportedKeyMaterial {
    pub raw_openssh_key: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KeyDescriptor {
    pub key_id: String,
    pub label: String,
    pub public_key_openssh: String,
    pub fingerprint_sha256: String,
    pub key_type: KeyType,
    pub is_hardware_backed: bool,
}

pub fn parse_and_validate_openssh_private_key(
    key_material: &ImportedKeyMaterial,
) -> Result<KeyDescriptor, CryptoError> {
    // Zeroizes passphrase immediately after derivation
    // Returns validated descriptor or typed error
    todo!()
}
```

### 2.2 UniFFI Interface

```rust
#[uniffi::export]
impl SshCoreVault {
    /// Imports an existing OpenSSH private key with optional passphrase.
    pub fn import_private_key(
        &self,
        label: String,
        armored_key: String,
        passphrase: Option<String>,
    ) -> Result<KeyDescriptor, SshCoreError>;

    /// Assembles an SSH `sk-` signature from what the authenticator returned: flags, counter and
    /// the raw signature (an SSH signature has no `clientDataJSON`; that is the WebAuthn shape).
    pub fn complete_sk_signature(
        &self,
        session_challenge: Vec<u8>,
        flags: u8,
        counter: u32,
        signature: Vec<u8>,
    ) -> Result<Vec<u8>, SshCoreError>;
}
```

### 2.3 Spike before any UI

With the chosen library, sign one SSH challenge with a real YubiKey and have a real `sshd` accept it (loopback `sshd` on a private port, as in `crates/core/tests/loopback_sshd.rs`). Until that works, section 2.2's second method is a design, not a promise.

---

## 3. iCloud Vault Sync Architecture

```mermaid
flowchart TD
    subgraph iPhone [iPhone Secure Enclave]
        PrivKeyA[Device Private Key A]
        KeyChainLocalA[Local Keychain DeviceOnly]
    end

    subgraph iPad [iPad Secure Enclave]
        PrivKeyB[Device Private Key B]
        KeyChainLocalB[Local Keychain DeviceOnly]
    end

    subgraph iCloud [Encrypted iCloud Key-Value / CloudKit Private Database]
        VaultMetadata[Servers, Profiles, Snippets, Public Keys]
    end

    iPhone -->|Syncs Profiles & PubKeys only| iCloud
    iPad -->|Syncs Profiles & PubKeys only| iCloud
    PrivKeyA -.->|NEVER LEAVES DEVICE| KeyChainLocalA
    PrivKeyB -.->|NEVER LEAVES DEVICE| KeyChainLocalB
```

- Each device maintains its own distinct Ed25519 hardware key in its local Keychain.
- Adding the iPad to a host means writing the iPad's public key into that host's `authorized_keys` (and `allowed_signers`). It is done per host with the pairing path of feature 44 (a password once, or a pasted command), not with the agent approval card.
- All server configurations (IP, port, username, launch profiles) sync instantaneously over Apple CloudKit.

---

## 4. Security & Threat Modeling (Hacker Gate)

| Threat | Mitigation |
| :--- | :--- |
| **Private Key Exposure in iCloud** | Private keys are marked `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and are explicitly excluded from the CloudKit sync payload. Only metadata and public keys sync. |
| **Passphrase Memory Leaks** | Rust Core wraps all imported key buffers in `ZeroizeOnDrop`. Memory is scrubbed with zeroes when parsed. |
| **Brute Force on Imported Keys** | Key derivation is bcrypt-pbkdf via the `ssh-key` routines, with a ceiling on the rounds count accepted from a file so a hostile file cannot stall the phone. |
| **iCloud trust transfer** | Fingerprint sync is off by default and says what it does; a changed fingerprint on the other device is shown as a change, never accepted silently. |
| **NFC Relay / Rogue FIDO2 Assertions** | The `flags` byte (user presence, and user verification where the host requires it) and the counter are checked, and the signature is made over the SSH session data, so it cannot be replayed on another handshake. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_import_valid_ed25519_unencrypted_key`.
   - `test_import_passphrase_protected_key_with_bcrypt_pbkdf` (a rounds count above the ceiling is refused).
   - `test_reject_invalid_passphrase_with_typed_error`.
   - `test_sk_signature_has_flags_counter_and_signature_only`.
   - `test_secure_enclave_key_is_p256`.
2. **Swift Platform Tests**:
   - CoreNFC YubiKey challenge-response flow mock.
   - CloudKit conflict resolution: last writer wins by timestamp for server metadata (vector clocks are not used).
   - Passphrase is passed as `Data` and cleared after use.

---

## Review (`rev`, 2026-10-20) - applied

Folded in above: library and own `sk-` assembly (first deliverable, with a real-`sshd` spike); the SSH signature shape; Secure Enclave is P-256; the iPad key goes through pairing; fingerprint sync is a default-off decision; bcrypt-pbkdf instead of Argon2; passphrase as `Data` with the limit stated; `bool` typo; last-writer-wins instead of vector clocks.

---

## 6. Implemented slice (2026-10-22)

**Key import** (`crates/core/src/keys.rs`). `import_private_key(armored, passphrase: Option<Vec<u8>>) -> ImportedKey` and `private_key_needs_passphrase(armored)`. The passphrase is `Data` on the Swift side and `Zeroizing` in Rust; the armored text is zeroized too. A bcrypt-pbkdf rounds count above `MAX_BCRYPT_ROUNDS` (500; `ssh-keygen` uses 16) is refused **before** any derivation (a crafted file with `u32::MAX` rounds is refused in well under a second, tested). No passphrase gives `KeyPassphraseRequired`, a wrong one `KeyPassphraseWrong`. Only Ed25519 is accepted, because the connection path signs with no RSA hash choice (`PrivateKeyWithHashAlg::new(key, None)` would mean SHA-1 for RSA, which current servers refuse) and ECDSA is not proved against a real `sshd`; other types get a message saying so. The Keychain stores the decrypted key (the item is the protection, `ThisDeviceOnly`); the passphrase is kept nowhere. The app derives off the main thread and a cancelled task stores nothing.

**Server sync** (`crates/core/src/vault_sync.rs`, `ios/Sources/SSHApp/Services/VaultSync.swift`). One synchronizable Keychain item (`com.sshapp.vault`) that iCloud Keychain carries end to end encrypted; no iCloud container, no entitlement. The core decides per entry: last writer wins by `updated_at_ms`; on a tie a deletion wins, then the larger payload, so two devices converge whichever is "local". Deletions are tombstones for 30 days. Remote entries are dropped when the id is malformed, the payload is over 16 KB, there are over 1000 entries, or the stamp is more than a day ahead (wrong clock, or an attempt to win every later edit).

What travels: name, host, port, user, and the pinned fingerprint **only** with its toggle (off by default). What does not: private keys, `keyId` (each device signs with its own key; the iPad's key is added to a host through pairing, section 3), the default flag, Files settings. A received server is checked again on the phone (`SyncedServer.isSane`: no leading `-`, no whitespace or control characters, length caps, port above 0). A differing fingerprint on the other device is shown as a conflict and never replaces the local pin. Triggers: start, becoming active, after a local change; the phone does not poll.

Known limits: a server added by hand on two devices has two different ids and shows twice (merge by address is a guess that could join two different hosts); launch profiles and snippets are wired (below); the sync item can be read by anything that holds the iCloud Keychain, which is the same trust as the account.

Tests: 8 Rust (import, wrong/missing passphrase, rounds ceiling, hostile file, wrong key type, garbage), 7 Rust (merge), 8 Swift (arrival without key choice, later edit wins and keeps the local key, deletions both ways, fingerprint toggle and conflict, hostile payloads, payload has no key data).

### Launch profiles and snippets (2026-10-23)

They travel in the same item under the id prefixes `p_` and `s_` (ids of at most 64 characters of `[A-Za-z0-9_-]`). Neither has an edit time, so `VaultCollectionEngine` (`VaultCollections.swift`) stamps a local edit when the payload differs from the last one seen, turns an id that disappeared into a tombstone, and leaves the winner to `mergeVault`. Two switches in Settings (on by default): launch profiles and snippets.

What does **not** leave the device: values of environment variables whose name contains KEY, TOKEN, SECRET, PASSWORD, PASSWD, CREDENTIAL or AUTH; any profile with a custom hook command (a command the host would run); snippets whose title or command matches the secret pattern of the snippet catalog. A received profile is checked again before it is used (lengths, no control characters, no leading `-` in the binary, `KEY=VALUE` environment names, no custom hook, the id inside equals the entry id) and keeps the secrets and the custom hook this device already has for the same id. A received snippet is limited in length and tags. Anything refused stays in the vault for devices that accept it and is never applied.

Known limit: someone who holds the iCloud Keychain can add a launch profile or snippet that the host would run when the owner starts it; that is the same trust as the account, and nothing starts without a tap.

Tests: 10 Swift (`VaultCollectionTests`): edit and deletion rules, no echo after applying, other prefixes untouched, secrets and custom hooks stay, hostile profiles and snippets refused.
