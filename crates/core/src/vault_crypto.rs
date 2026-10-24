//! Cryptography of the encrypted vault and of encrypted backups (feature 56).
//!
//! A passphrase typed by the user is stretched with Argon2id and wraps a random 32 byte vault key.
//! Entries that travel through iCloud are sealed with keys derived from the vault key, so the
//! account holder who can read the synced Keychain still sees only ciphertext. Every purpose gets
//! its own HKDF label; a key made for one purpose is never used for another.
//!
//! Layout of a passphrase container (vault or backup):
//! `magic(4) | version(1) | kdf(1) | m_kib(4) | t(4) | p(1) | salt(16) | created_at_ms(8)`
//! followed by `nonce(12) | ChaCha20-Poly1305(plaintext)`; the header is the AEAD associated data,
//! so changing any byte of it breaks the tag.
//!
//! Key buffers are zeroized on drop inside this module. Values handed over the FFI boundary are
//! plain byte arrays; the caller must not keep them longer than needed.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngExt;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::error::SshCoreError;
use crate::vault_sync::{valid_entry_id, VaultEntry, MAX_PAYLOAD_BYTES};

const VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 4 + 1 + 1 + 4 + 4 + 1 + SALT_LEN + 8;
pub const VAULT_KEY_LEN: usize = 32;

/// Default cost, about a second on a recent iPhone. Tuned on the device later; the floors below
/// are what an importer accepts.
pub const DEFAULT_M_KIB: u32 = 64 * 1024;
pub const DEFAULT_T: u32 = 3;
pub const DEFAULT_P: u32 = 1;
pub const MIN_M_KIB: u32 = 32 * 1024;
pub const MIN_T: u32 = 2;
/// A hostile file must not make the phone allocate gigabytes or compute for minutes.
pub const MAX_M_KIB: u32 = 256 * 1024;
pub const MAX_T: u32 = 10;
pub const MAX_P: u32 = 4;

/// Passphrases shorter than this (in characters) are refused.
pub const MIN_PASSPHRASE_CHARS: usize = 12;
/// Sealed entries are padded to a multiple of this so their size does not give a key's length away.
const PAD_TO: usize = 1024;

const LABEL_VAULT_WRAP: &[u8] = b"ssh-app/vault/wrap";
const LABEL_BACKUP_ENC: &[u8] = b"ssh-app/backup/enc";
const LABEL_ENTRY_ENC: &[u8] = b"ssh-app/vault/entry-enc";
const LABEL_ENTRY_SIG: &[u8] = b"ssh-app/vault/entry-sig";
const AAD_ENTRY: &[u8] = b"ssh-app/vault/entry\0";
const MAC_ENTRY: &[u8] = b"ssh-app/vault/entry-mac";

/// What a container protects; it selects the magic bytes and the HKDF label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Vault,
    /// Used by the encrypted backup file (spec 56 section 5).
    Backup,
}

impl Kind {
    fn magic(self) -> &'static [u8; 4] {
        match self {
            Kind::Vault => b"SSHV",
            Kind::Backup => b"SSHB",
        }
    }
    fn label(self) -> &'static [u8] {
        match self {
            Kind::Vault => LABEL_VAULT_WRAP,
            Kind::Backup => LABEL_BACKUP_ENC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    pub(crate) const DEFAULT: KdfParams = KdfParams {
        m_kib: DEFAULT_M_KIB,
        t: DEFAULT_T,
        p: DEFAULT_P,
    };

    fn check(&self) -> Result<(), SshCoreError> {
        if self.m_kib < MIN_M_KIB || self.t < MIN_T || self.p == 0 {
            return Err(invalid("the key derivation settings are too weak"));
        }
        if self.m_kib > MAX_M_KIB || self.t > MAX_T || self.p > MAX_P {
            return Err(invalid("the key derivation settings are too demanding"));
        }
        Ok(())
    }
}

fn invalid(reason: &str) -> SshCoreError {
    SshCoreError::VaultDataInvalid {
        reason: reason.to_string(),
    }
}

struct Header {
    params: KdfParams,
    salt: [u8; SALT_LEN],
    created_at_ms: u64,
}

fn encode_header(kind: Kind, h: &Header) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(kind.magic());
    out.push(VERSION);
    out.push(KDF_ARGON2ID);
    out.extend_from_slice(&h.params.m_kib.to_be_bytes());
    out.extend_from_slice(&h.params.t.to_be_bytes());
    out.push(h.params.p as u8);
    out.extend_from_slice(&h.salt);
    out.extend_from_slice(&h.created_at_ms.to_be_bytes());
    out
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn parse_header(kind: Kind, data: &[u8]) -> Result<Header, SshCoreError> {
    if data.len() < HEADER_LEN + NONCE_LEN + TAG_LEN {
        return Err(invalid("the data is too short"));
    }
    if &data[..4] != kind.magic() {
        return Err(invalid("this is not the expected kind of file"));
    }
    if data[4] != VERSION {
        return Err(invalid("unsupported version"));
    }
    if data[5] != KDF_ARGON2ID {
        return Err(invalid("unsupported key derivation"));
    }
    let params = KdfParams {
        m_kib: u32_at(data, 6),
        t: u32_at(data, 10),
        p: u32::from(data[14]),
    };
    // Checked before the (expensive) derivation runs.
    params.check()?;
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&data[15..15 + SALT_LEN]);
    let mut at = [0u8; 8];
    at.copy_from_slice(&data[15 + SALT_LEN..HEADER_LEN]);
    Ok(Header {
        params,
        salt,
        created_at_ms: u64::from_be_bytes(at),
    })
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::rng().fill(&mut b[..]);
    b
}

fn hkdf_key(ikm: &[u8], label: &[u8]) -> Result<Zeroizing<[u8; 32]>, SshCoreError> {
    let mut out = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(None, ikm)
        .expand(label, &mut out[..])
        .map_err(|_| invalid("key derivation failed"))?;
    Ok(out)
}

fn wrap_key(
    kind: Kind,
    passphrase: &[u8],
    salt: &[u8],
    params: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, SshCoreError> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(32))
        .map_err(|_| invalid("the key derivation settings are not valid"))?;
    let mut raw = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(passphrase, salt, &mut raw[..])
        .map_err(|_| invalid("key derivation failed"))?;
    hkdf_key(&raw[..], kind.label())
}

/// Encrypts `plaintext` under `passphrase` into a container of `kind`.
pub(crate) fn seal_container(
    kind: Kind,
    passphrase: &[u8],
    plaintext: &[u8],
    created_at_ms: u64,
    params: KdfParams,
) -> Result<Vec<u8>, SshCoreError> {
    params.check()?;
    let header = Header {
        params,
        salt: random_bytes::<SALT_LEN>(),
        created_at_ms,
    };
    seal_with_header(kind, passphrase, plaintext, &header)
}

fn seal_with_header(
    kind: Kind,
    passphrase: &[u8],
    plaintext: &[u8],
    header: &Header,
) -> Result<Vec<u8>, SshCoreError> {
    let key = wrap_key(kind, passphrase, &header.salt, header.params)?;
    let head = encode_header(kind, header);
    let nonce = random_bytes::<NONCE_LEN>();
    let ct = ChaCha20Poly1305::new(Key::from_slice(&key[..]))
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &head,
            },
        )
        .map_err(|_| invalid("encryption failed"))?;
    let mut out = head;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a container of `kind`. A wrong passphrase and a damaged container give the same error.
pub(crate) fn open_container(
    kind: Kind,
    data: &[u8],
    passphrase: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SshCoreError> {
    let header = parse_header(kind, data)?;
    let key = wrap_key(kind, passphrase, &header.salt, header.params)?;
    let nonce = &data[HEADER_LEN..HEADER_LEN + NONCE_LEN];
    ChaCha20Poly1305::new(Key::from_slice(&key[..]))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &data[HEADER_LEN + NONCE_LEN..],
                aad: &data[..HEADER_LEN],
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| SshCoreError::VaultWrongPassphrase)
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultCreated {
    /// Goes to iCloud.
    pub container: Vec<u8>,
    /// Stays on the device (local Keychain, this device only).
    pub vault_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultInfo {
    pub created_at_ms: u64,
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultOpened {
    pub updated_at_ms: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PassphraseCheck {
    /// False: the passphrase is refused (too short or trivially repetitive).
    pub acceptable: bool,
    /// Shown to the user; may be non-empty even when `acceptable`.
    pub warnings: Vec<String>,
}

/// Judges a passphrase. The minimum length is enforced; the rest are warnings.
#[uniffi::export]
pub fn passphrase_strength(passphrase: Vec<u8>) -> PassphraseCheck {
    let Ok(text) = std::str::from_utf8(&passphrase) else {
        return PassphraseCheck {
            acceptable: false,
            warnings: vec!["The passphrase is not valid text.".into()],
        };
    };
    let chars: Vec<char> = text.chars().collect();
    let mut warnings = Vec::new();
    let mut acceptable = true;
    if chars.len() < MIN_PASSPHRASE_CHARS {
        acceptable = false;
        warnings.push(format!("Use at least {MIN_PASSPHRASE_CHARS} characters."));
    }
    let mut unique: Vec<char> = chars.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() < 5 {
        acceptable = false;
        warnings.push("The passphrase repeats a few characters.".into());
    }
    let classes = [
        chars.iter().any(|c| c.is_lowercase()),
        chars.iter().any(|c| c.is_uppercase()),
        chars.iter().any(|c| c.is_numeric()),
        chars
            .iter()
            .any(|c| !c.is_alphanumeric() && !c.is_whitespace()),
        chars.iter().any(|c| c.is_whitespace()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if chars.len() < 16 && classes < 2 {
        warnings.push("Mix kinds of characters or make it longer.".into());
    }
    let lower = text.to_lowercase();
    for common in ["password", "qwerty", "12345678", "letmein", "11111111"] {
        if lower.contains(common) {
            warnings.push("It contains a very common word or sequence.".into());
            break;
        }
    }
    PassphraseCheck {
        acceptable,
        warnings,
    }
}

/// Compares two typed passphrases (the confirmation field) without an early exit.
#[uniffi::export]
pub fn passphrases_match(a: Vec<u8>, b: Vec<u8>) -> bool {
    let a = Zeroizing::new(a);
    let b = Zeroizing::new(b);
    a.len() == b.len() && bool::from(a.ct_eq(&b))
}

pub(crate) fn require_acceptable(passphrase: &[u8]) -> Result<(), SshCoreError> {
    if passphrase_strength(passphrase.to_vec()).acceptable {
        Ok(())
    } else {
        Err(SshCoreError::InvalidInput {
            reason: format!("the passphrase must have at least {MIN_PASSPHRASE_CHARS} characters"),
        })
    }
}

fn vault_key_of(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, SshCoreError> {
    if bytes.len() != VAULT_KEY_LEN {
        return Err(invalid("the vault key has the wrong length"));
    }
    Ok(Zeroizing::new(bytes.to_vec()))
}

/// Creates a vault: a fresh vault key wrapped by `passphrase`.
#[uniffi::export]
pub fn vault_create(passphrase: Vec<u8>, created_at_ms: u64) -> Result<VaultCreated, SshCoreError> {
    let passphrase = Zeroizing::new(passphrase);
    require_acceptable(&passphrase)?;
    let key = Zeroizing::new(random_bytes::<VAULT_KEY_LEN>());
    let container = seal_container(
        Kind::Vault,
        &passphrase,
        &key[..],
        created_at_ms,
        KdfParams::DEFAULT,
    )?;
    Ok(VaultCreated {
        container,
        vault_key: key.to_vec(),
    })
}

/// Unwraps the vault key of a container with the user's passphrase.
#[uniffi::export]
pub fn vault_unlock(container: Vec<u8>, passphrase: Vec<u8>) -> Result<Vec<u8>, SshCoreError> {
    let passphrase = Zeroizing::new(passphrase);
    let key = open_container(Kind::Vault, &container, &passphrase)?;
    Ok(vault_key_of(&key)?.to_vec())
}

/// Wraps the same vault key with a new passphrase (new salt); entries stay valid.
#[uniffi::export]
pub fn vault_rewrap(
    container: Vec<u8>,
    old_passphrase: Vec<u8>,
    new_passphrase: Vec<u8>,
) -> Result<Vec<u8>, SshCoreError> {
    let old = Zeroizing::new(old_passphrase);
    let new = Zeroizing::new(new_passphrase);
    require_acceptable(&new)?;
    let header = parse_header(Kind::Vault, &container)?;
    let key = open_container(Kind::Vault, &container, &old)?;
    vault_key_of(&key)?;
    seal_container(
        Kind::Vault,
        &new,
        &key[..],
        header.created_at_ms,
        KdfParams::DEFAULT,
    )
}

/// Reads a container's header without a passphrase (to compare two containers, or to refuse one).
#[uniffi::export]
pub fn vault_container_info(container: Vec<u8>) -> Result<VaultInfo, SshCoreError> {
    let h = parse_header(Kind::Vault, &container)?;
    Ok(VaultInfo {
        created_at_ms: h.created_at_ms,
        m_kib: h.params.m_kib,
        t: h.params.t,
        p: h.params.p,
    })
}

fn entry_aad(id: &str, updated_at_ms: u64) -> Vec<u8> {
    let mut aad = AAD_ENTRY.to_vec();
    aad.extend_from_slice(id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&updated_at_ms.to_be_bytes());
    aad
}

/// Seals one tier B payload. Layout: `version(1) | updated_at(8) | nonce(12) | ciphertext`, where
/// the plaintext is `len(4) | payload | zero padding` up to a multiple of 1 KB. The id and the
/// timestamp are authenticated, so an entry cannot be moved under another id or given a newer time.
#[uniffi::export]
pub fn vault_seal_entry(
    vault_key: Vec<u8>,
    id: String,
    updated_at_ms: u64,
    payload: Vec<u8>,
) -> Result<Vec<u8>, SshCoreError> {
    let payload = Zeroizing::new(payload);
    let vk = vault_key_of(&vault_key)?;
    if !valid_entry_id(&id) {
        return Err(invalid("the entry id is not valid"));
    }
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid("the entry is too large"));
    }
    let key = hkdf_key(&vk, LABEL_ENTRY_ENC)?;
    let mut plain = Zeroizing::new(Vec::with_capacity(PAD_TO));
    plain.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    plain.extend_from_slice(&payload);
    let padded = plain.len().div_ceil(PAD_TO) * PAD_TO;
    plain.resize(padded, 0);
    let nonce = random_bytes::<NONCE_LEN>();
    let ct = ChaCha20Poly1305::new(Key::from_slice(&key[..]))
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &entry_aad(&id, updated_at_ms),
            },
        )
        .map_err(|_| invalid("encryption failed"))?;
    let mut out = vec![VERSION];
    out.extend_from_slice(&updated_at_ms.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens a sealed entry for `id`; fails if the id, the timestamp or any byte was changed.
#[uniffi::export]
pub fn vault_open_entry(
    vault_key: Vec<u8>,
    id: String,
    sealed: Vec<u8>,
) -> Result<VaultOpened, SshCoreError> {
    let vk = vault_key_of(&vault_key)?;
    if !valid_entry_id(&id) {
        return Err(invalid("the entry id is not valid"));
    }
    if sealed.len() < 1 + 8 + NONCE_LEN + TAG_LEN + 4 || sealed[0] != VERSION {
        return Err(invalid("the entry is malformed or of an unknown version"));
    }
    // A sealed entry is at most the largest payload, its length prefix, padding and tag.
    if sealed.len() > 1 + 8 + NONCE_LEN + TAG_LEN + MAX_PAYLOAD_BYTES + 4 + PAD_TO {
        return Err(invalid("the entry is too large"));
    }
    let mut at = [0u8; 8];
    at.copy_from_slice(&sealed[1..9]);
    let updated_at_ms = u64::from_be_bytes(at);
    let key = hkdf_key(&vk, LABEL_ENTRY_ENC)?;
    let plain = ChaCha20Poly1305::new(Key::from_slice(&key[..]))
        .decrypt(
            Nonce::from_slice(&sealed[9..9 + NONCE_LEN]),
            Payload {
                msg: &sealed[9 + NONCE_LEN..],
                aad: &entry_aad(&id, updated_at_ms),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| SshCoreError::VaultWrongPassphrase)?;
    if plain.len() < 4 {
        return Err(invalid("the entry is malformed"));
    }
    let len = u32_at(&plain, 0) as usize;
    if len > plain.len() - 4 || len > MAX_PAYLOAD_BYTES {
        return Err(invalid("the entry is malformed"));
    }
    Ok(VaultOpened {
        updated_at_ms,
        payload: plain[4..4 + len].to_vec(),
    })
}

fn entry_mac(vault_key: &[u8], e: &VaultEntry) -> Result<Hmac<Sha256>, SshCoreError> {
    let vk = vault_key_of(vault_key)?;
    let key = hkdf_key(&vk, LABEL_ENTRY_SIG)?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[..])
        .map_err(|_| invalid("key derivation failed"))?;
    mac.update(MAC_ENTRY);
    for field in [e.id.as_bytes(), e.payload.as_bytes()] {
        mac.update(&(field.len() as u32).to_be_bytes());
        mac.update(field);
    }
    mac.update(&e.updated_at_ms.to_be_bytes());
    mac.update(&[u8::from(e.deleted)]);
    Ok(mac)
}

/// MAC of a tier A entry, so an entry written by someone without the vault key can be recognised.
#[uniffi::export]
pub fn entry_sign(vault_key: Vec<u8>, entry: VaultEntry) -> Result<Vec<u8>, SshCoreError> {
    Ok(entry_mac(&vault_key, &entry)?
        .finalize()
        .into_bytes()
        .to_vec())
}

/// Constant-time check of an entry's MAC.
#[uniffi::export]
pub fn entry_verify(vault_key: Vec<u8>, entry: VaultEntry, mac: Vec<u8>) -> bool {
    match entry_mac(&vault_key, &entry) {
        Ok(m) => m.verify_slice(&mac).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &[u8] = b"correct horse battery staple";

    /// The floor is the cheapest setting an importer accepts; tests use it to stay fast.
    const FAST: KdfParams = KdfParams {
        m_kib: MIN_M_KIB,
        t: MIN_T,
        p: 1,
    };

    fn key() -> Vec<u8> {
        vec![7u8; VAULT_KEY_LEN]
    }

    fn entry(id: &str, at: u64, payload: &str) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            updated_at_ms: at,
            deleted: false,
            payload: payload.into(),
        }
    }

    #[test]
    fn a_container_opens_with_the_right_passphrase_only() {
        let c = seal_container(Kind::Vault, PHRASE, b"secret key bytes", 5, FAST).unwrap();
        assert_eq!(
            &open_container(Kind::Vault, &c, PHRASE).unwrap()[..],
            b"secret key bytes"
        );
        assert!(matches!(
            open_container(Kind::Vault, &c, b"a different passphrase"),
            Err(SshCoreError::VaultWrongPassphrase)
        ));
    }

    #[test]
    fn every_header_and_body_byte_is_authenticated() {
        let c = seal_container(Kind::Vault, PHRASE, b"payload", 5, FAST).unwrap();
        for i in 0..c.len() {
            let mut bad = c.clone();
            bad[i] ^= 1;
            assert!(
                open_container(Kind::Vault, &bad, PHRASE).is_err(),
                "flipping byte {i} must not open"
            );
        }
    }

    #[test]
    fn a_backup_container_does_not_open_as_a_vault() {
        let c = seal_container(Kind::Backup, PHRASE, b"x", 1, FAST).unwrap();
        assert!(matches!(
            open_container(Kind::Vault, &c, PHRASE),
            Err(SshCoreError::VaultDataInvalid { .. })
        ));
        assert!(open_container(Kind::Backup, &c, PHRASE).is_ok());
    }

    #[test]
    fn weak_and_excessive_kdf_settings_are_refused_before_the_derivation() {
        let ok = seal_container(Kind::Vault, PHRASE, b"x", 1, FAST).unwrap();
        let mut weak = ok.clone();
        weak[6..10].copy_from_slice(&(MIN_M_KIB - 1).to_be_bytes());
        let mut heavy = ok.clone();
        heavy[6..10].copy_from_slice(&(MAX_M_KIB + 1).to_be_bytes());
        let mut many = ok.clone();
        many[10..14].copy_from_slice(&(MAX_T + 1).to_be_bytes());
        let mut lanes = ok;
        lanes[14] = 0;
        for bad in [weak, heavy, many, lanes] {
            let started = std::time::Instant::now();
            assert!(matches!(
                open_container(Kind::Vault, &bad, PHRASE),
                Err(SshCoreError::VaultDataInvalid { .. })
            ));
            assert!(
                started.elapsed().as_millis() < 200,
                "refused before the KDF"
            );
        }
        assert!(seal_container(
            Kind::Vault,
            PHRASE,
            b"x",
            1,
            KdfParams {
                m_kib: 1024,
                t: 1,
                p: 1
            }
        )
        .is_err());
    }

    #[test]
    fn short_truncated_and_wrong_version_data_is_refused() {
        let c = seal_container(Kind::Vault, PHRASE, b"x", 1, FAST).unwrap();
        assert!(open_container(Kind::Vault, &c[..HEADER_LEN], PHRASE).is_err());
        assert!(open_container(Kind::Vault, &[], PHRASE).is_err());
        let mut v2 = c.clone();
        v2[4] = 2;
        assert!(matches!(
            open_container(Kind::Vault, &v2, PHRASE),
            Err(SshCoreError::VaultDataInvalid { .. })
        ));
        let mut kdf = c;
        kdf[5] = 9;
        assert!(open_container(Kind::Vault, &kdf, PHRASE).is_err());
    }

    #[test]
    fn create_unlock_rewrap_keep_the_same_vault_key() {
        let made = vault_create(PHRASE.to_vec(), 42).unwrap();
        assert_eq!(made.vault_key.len(), VAULT_KEY_LEN);
        assert_eq!(
            vault_unlock(made.container.clone(), PHRASE.to_vec()).unwrap(),
            made.vault_key
        );
        let info = vault_container_info(made.container.clone()).unwrap();
        assert_eq!(
            (info.created_at_ms, info.m_kib, info.t, info.p),
            (42, DEFAULT_M_KIB, DEFAULT_T, DEFAULT_P)
        );

        let rewrapped = vault_rewrap(
            made.container.clone(),
            PHRASE.to_vec(),
            b"another long passphrase".to_vec(),
        )
        .unwrap();
        assert_eq!(
            vault_unlock(rewrapped.clone(), b"another long passphrase".to_vec()).unwrap(),
            made.vault_key
        );
        assert!(vault_unlock(rewrapped.clone(), PHRASE.to_vec()).is_err());
        assert_eq!(
            vault_container_info(rewrapped.clone())
                .unwrap()
                .created_at_ms,
            42
        );
        assert_ne!(rewrapped[15..31], made.container[15..31], "new salt");
        assert!(vault_rewrap(
            made.container,
            b"nope nope nope nope".to_vec(),
            b"another long passphrase".to_vec()
        )
        .is_err());
    }

    #[test]
    fn a_vault_is_not_created_with_a_weak_passphrase() {
        assert!(matches!(
            vault_create(b"short".to_vec(), 1),
            Err(SshCoreError::InvalidInput { .. })
        ));
        assert!(vault_create(b"aaaaaaaaaaaaaaaaaaaa".to_vec(), 1).is_err());
        assert!(vault_create(vec![0xff; 20], 1).is_err());
    }

    #[test]
    fn passphrase_checks() {
        assert!(!passphrase_strength(b"short".to_vec()).acceptable);
        assert!(passphrase_strength(PHRASE.to_vec()).acceptable);
        let weak = passphrase_strength(b"password1234".to_vec());
        assert!(
            weak.acceptable && !weak.warnings.is_empty(),
            "common word warns but is allowed"
        );
        assert!(passphrase_strength("пароль для сейфа 2026".as_bytes().to_vec()).acceptable);
        assert!(passphrases_match(b"abc".to_vec(), b"abc".to_vec()));
        assert!(!passphrases_match(b"abc".to_vec(), b"abd".to_vec()));
        assert!(!passphrases_match(b"abc".to_vec(), b"abcd".to_vec()));
    }

    #[test]
    fn an_entry_round_trips_and_is_bound_to_its_id_and_time() {
        let sealed =
            vault_seal_entry(key(), "k_one".into(), 99, b"-----BEGIN KEY-----".to_vec()).unwrap();
        let opened = vault_open_entry(key(), "k_one".into(), sealed.clone()).unwrap();
        assert_eq!(
            (opened.updated_at_ms, opened.payload.as_slice()),
            (99, &b"-----BEGIN KEY-----"[..])
        );
        assert!(
            vault_open_entry(key(), "k_two".into(), sealed.clone()).is_err(),
            "moved under another id"
        );
        let mut newer = sealed.clone();
        newer[8] ^= 1;
        assert!(
            vault_open_entry(key(), "k_one".into(), newer).is_err(),
            "given another time"
        );
        assert!(
            vault_open_entry(vec![8u8; 32], "k_one".into(), sealed.clone()).is_err(),
            "other key"
        );
        for i in 0..sealed.len() {
            let mut bad = sealed.clone();
            bad[i] ^= 0x80;
            assert!(
                vault_open_entry(key(), "k_one".into(), bad).is_err(),
                "byte {i}"
            );
        }
    }

    #[test]
    fn sealed_sizes_are_padded_and_nonces_never_repeat() {
        let overhead = 1 + 8 + NONCE_LEN + TAG_LEN;
        let small = vault_seal_entry(key(), "a".into(), 1, vec![1; 10]).unwrap();
        let medium = vault_seal_entry(key(), "a".into(), 1, vec![1; 900]).unwrap();
        assert_eq!(small.len(), overhead + PAD_TO);
        assert_eq!(small.len(), medium.len(), "different lengths look alike");
        let big = vault_seal_entry(key(), "a".into(), 1, vec![1; 1100]).unwrap();
        assert_eq!(big.len(), overhead + 2 * PAD_TO);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            let s = vault_seal_entry(key(), "a".into(), 1, vec![1; 10]).unwrap();
            assert!(seen.insert(s[9..9 + NONCE_LEN].to_vec()), "nonce reused");
        }
    }

    #[test]
    fn entries_with_bad_ids_keys_or_sizes_are_refused() {
        assert!(vault_seal_entry(vec![1; 5], "a".into(), 1, vec![]).is_err());
        assert!(vault_seal_entry(key(), "bad id!".into(), 1, vec![]).is_err());
        assert!(vault_seal_entry(key(), "".into(), 1, vec![]).is_err());
        assert!(vault_seal_entry(key(), "a".into(), 1, vec![0; MAX_PAYLOAD_BYTES + 1]).is_err());
        assert!(vault_seal_entry(key(), "a".into(), 1, vec![0; MAX_PAYLOAD_BYTES]).is_ok());
        assert!(vault_open_entry(key(), "a".into(), vec![]).is_err());
        assert!(vault_open_entry(key(), "a".into(), vec![1; 10_000_000]).is_err());
    }

    #[test]
    fn the_mac_covers_every_field_and_the_key() {
        let e = entry("srv_1", 10, "{\"host\":\"a\"}");
        let mac = entry_sign(key(), e.clone()).unwrap();
        assert!(entry_verify(key(), e.clone(), mac.clone()));
        assert!(!entry_verify(vec![8u8; 32], e.clone(), mac.clone()));
        let mut other = e.clone();
        other.id = "srv_2".into();
        assert!(!entry_verify(key(), other, mac.clone()));
        let mut other = e.clone();
        other.updated_at_ms = 11;
        assert!(!entry_verify(key(), other, mac.clone()));
        let mut other = e.clone();
        other.deleted = true;
        assert!(!entry_verify(key(), other, mac.clone()));
        let mut other = e.clone();
        other.payload.push(' ');
        assert!(!entry_verify(key(), other, mac.clone()));
        assert!(!entry_verify(key(), e.clone(), mac[..31].to_vec()));
        assert!(!entry_verify(key(), e, vec![]));
        // Length prefixes: moving a byte between id and payload changes the MAC.
        let a = entry_sign(key(), entry("ab", 1, "c")).unwrap();
        let b = entry_sign(key(), entry("a", 1, "bc")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn purposes_use_different_keys() {
        let ikm = [3u8; 32];
        let labels = [
            LABEL_VAULT_WRAP,
            LABEL_BACKUP_ENC,
            LABEL_ENTRY_ENC,
            LABEL_ENTRY_SIG,
        ];
        let keys: Vec<_> = labels.iter().map(|l| *hkdf_key(&ikm, l).unwrap()).collect();
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                assert_ne!(keys[i], keys[j]);
            }
        }
    }
}
