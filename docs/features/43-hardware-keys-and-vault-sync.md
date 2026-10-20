# Feature 43: Hardware Security Keys (FIDO2 / YubiKey), Key Import & iCloud Keychain Sync

- **Slug**: `hardware-keys-and-vault-sync`
- **Target Release**: Milestone 32 (Enterprise Security & Cross-Device Key Management)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (cryptographic key import, FIDO2/WebAuthn assertions, iCloud sync encryption, Secure Enclave)
- **Status**: Not ready: FIDO2 and Secure Enclave plans are not feasible as written (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
1. **Hardware Token Enforcement**: Many corporate and production environments strictly forbid pure software SSH keys, mandating hardware security tokens (`ssh-ed25519-sk` or `ecdsa-sk` via YubiKey or FIDO2 authenticator).
2. **Key Migration Friction**: Developers already have established SSH keys (`~/.ssh/id_ed25519` or 1Password SSH Agent) on their laptops. Generating an isolated key on the phone and manually pasting it into `authorized_keys` across 10 servers creates high friction.
3. **Multi-Device Sync**: Developers using both an iPhone and an iPad currently have to re-enter host configurations, Tailscale IPs, port rules, and snippets separately on each device.

### 1.2 In Scope
- Support for OpenSSH FIDO2 security keys (`sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com`) via NFC and USB-C hardware authenticators (YubiKey 5 Series).
- Secure private key import:
  - Standard OpenSSH format (`-----BEGIN OPENSSH PRIVATE KEY-----`).
  - Passphrase-encrypted keys decrypted locally in memory using `zeroize`-protected buffers.
  - Apple Secure Enclave key generation option (`kSecAttrTokenIDSecureEnclave`).
- End-to-End Encrypted iCloud Keychain Vault Sync:
  - Syncs server profiles, connection metadata, launch profiles, and snippets.
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
    pub is_hardware_backed: Bool,
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

    /// Handles a FIDO2 WebAuthn assertion received from iOS CoreNFC/AuthenticationServices.
    pub fn complete_fido2_signature(
        &self,
        session_challenge: Vec<u8>,
        auth_data: Vec<u8>,
        client_data_json: Vec<u8>,
        signature: Vec<u8>,
    ) -> Result<Vec<u8>, SshCoreError>;
}
```

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
- When an iPhone connects to a host, it authorizes the host. If an iPad is added, the user can approve the iPad's public key with 1 tap from the iPhone approval card.
- All server configurations (IP, port, username, launch profiles) sync instantaneously over Apple CloudKit.

---

## 4. Security & Threat Modeling (Hacker Gate)

| Threat | Mitigation |
| :--- | :--- |
| **Private Key Exposure in iCloud** | Private keys are marked `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and are explicitly excluded from the CloudKit sync payload. Only metadata and public keys sync. |
| **Passphrase Memory Leaks** | Rust Core wraps all imported key buffers in `ZeroizeOnDrop`. Memory is scrubbed with zeroes when parsed. |
| **Brute Force on Imported Keys** | If an imported key is passphrase-protected, PBKDF2 / Argon2 / bcrypt key derivation is performed via standard `ssh-key` cryptographic routines with resource limits to prevent denial of service. |
| **NFC Relay / Rogue FIDO2 Assertions** | FIDO2 user presence (`UP` flag) and user verification (`UV` flag) are strictly verified against the challenge bound to the specific SSH session handshake. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_import_valid_ed25519_unencrypted_key`.
   - `test_import_passphrase_protected_key_with_argon2`.
   - `test_reject_invalid_passphrase_with_typed_error`.
   - `test_verify_fido2_signature_flags`.
2. **Swift Platform Tests**:
   - CoreNFC YubiKey challenge-response flow mock.
   - CloudKit conflict resolution (last-write-wins with vector clocks for server metadata).

---

## Review (`rev`, 2026-10-20)

- [High] iOS gives no raw CTAP2 over NFC/USB-C, and WebAuthn cannot use the `ssh:` relying party. A third-party library (for example YubiKit) and our own `sk-` signature assembly are needed; neither is in the spec.
- [High] `complete_fido2_signature(auth_data, client_data_json, ...)` is the WebAuthn shape. An SSH `sk-` signature has no `clientDataJSON`; the format is flags + counter + signature.
- [High] Secure Enclave keys are P-256 only, not Ed25519. That is a different key type with its own signing path in russh.
- [High] Adding the iPad's key "with one tap" needs writing its key into `authorized_keys` and `allowed_signers` of every host. Define it; it is not the agent approval card.
- [Medium] Syncing pinned host fingerprints through iCloud transfers trust; list it as a decision.
- [Medium] OpenSSH keys use bcrypt-pbkdf, not Argon2; the Argon2 test cannot exist. High rounds take seconds on a phone.
- [Medium] A Swift `String` passphrase cannot be zeroized; use `Data` and say what is out of reach.
- [Low] `is_hardware_backed: Bool` is a typo for `bool`; vector clocks are overkill.
