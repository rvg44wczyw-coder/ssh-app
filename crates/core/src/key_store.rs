//! Private SSH keys live here and nowhere else (owner's rule, AGENTS.md): the core generates,
//! imports, stores and uses them, and the app holds only a [`KeyRef`]. Every secret that crosses the
//! FFI is bytes; the key text exists in the core only inside `Zeroizing`.
//!
//! Storage is the Keychain (same service and account names the Swift side used before, so keys
//! written earlier are read in place). Tests and non-Apple builds use an in-memory store, so no test
//! can touch a real Keychain.

use crate::error::SshCoreError;
use crate::keys::parse_private_key;
use argon2::{Algorithm as Argon, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngExt;
use ssh_key::{Algorithm, HashAlg};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use zeroize::Zeroizing;

const KEY_SERVICE: &str = "com.sshapp.keys";
/// Copies that need the owner's presence to be read (the Files extension).
const GUARDED_SERVICE: &str = "com.sshapp.filekeys";
const SEAL_MAGIC: &[u8; 5] = b"SSHK1";
/// Shortest password that may seal an exported key (the app asks for 12 characters too): the file
/// can be attacked offline for as long as it exists.
const MIN_SEAL_PASSWORD: usize = 12;
/// What the owner has to type to export a key in the clear.
pub const EXPORT_CONFIRMATION: &str = "EXPORT";

/// Which key: `id == None` is the original default key. `guarded` reads the copy that needs Face ID
/// or the passcode (Files extension).
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct KeyRef {
    pub id: Option<String>,
    pub guarded: bool,
}

/// The public side of a key: not secret.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyInfo {
    pub public_key_openssh: String,
    pub fingerprint_sha256: String,
}

/// The three acknowledgements an export in the clear needs; the app asks for each separately.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExportAcks {
    /// "Whoever has this key can log in to every server that trusts it."
    pub understands_access: bool,
    /// "Text on screen, in the clipboard or in a file can be copied by other apps and backups."
    pub understands_exposure: bool,
    /// The word [`EXPORT_CONFIRMATION`] typed by the owner.
    pub typed_confirmation: String,
}

trait Backend: Send + Sync {
    fn get(&self, service: &str, account: &str)
        -> Result<Option<Zeroizing<Vec<u8>>>, SshCoreError>;
    fn put(
        &self,
        service: &str,
        account: &str,
        data: &[u8],
        guarded: bool,
    ) -> Result<(), SshCoreError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), SshCoreError>;
    fn delete_service(&self, service: &str) -> Result<(), SshCoreError>;
}

#[derive(Default)]
#[cfg_attr(
    not(any(test, feature = "test-support", not(target_vendor = "apple"))),
    allow(dead_code)
)]
struct Memory(Mutex<HashMap<(String, String), Vec<u8>>>);

impl Backend for Memory {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, SshCoreError> {
        let map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(map
            .get(&(service.to_string(), account.to_string()))
            .map(|v| Zeroizing::new(v.clone())))
    }
    fn put(
        &self,
        service: &str,
        account: &str,
        data: &[u8],
        _guarded: bool,
    ) -> Result<(), SshCoreError> {
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.insert((service.to_string(), account.to_string()), data.to_vec());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SshCoreError> {
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
    fn delete_service(&self, service: &str) -> Result<(), SshCoreError> {
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|(s, _), _| s != service);
        Ok(())
    }
}

#[cfg(all(target_vendor = "apple", not(feature = "test-support")))]
mod keychain {
    use super::{Backend, SshCoreError, Zeroizing};
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        delete_generic_password_options, generic_password, set_generic_password_options,
        AccessControlOptions, PasswordOptions,
    };
    use security_framework_sys::item::{kSecAttrService, kSecClass, kSecClassGenericPassword};
    use security_framework_sys::keychain_item::SecItemDelete;

    /// `errSecItemNotFound`
    const NOT_FOUND: i32 = -25300;

    pub(super) struct Keychain {
        pub group: Option<String>,
    }

    fn err(what: &str, e: &security_framework::base::Error) -> SshCoreError {
        SshCoreError::IoError {
            reason: format!("Keychain {what} failed (code {})", e.code()),
        }
    }

    impl Keychain {
        fn options(&self, service: &str, account: &str) -> PasswordOptions {
            let mut o = PasswordOptions::new_generic_password(service, account);
            if let Some(g) = &self.group {
                o.set_access_group(g);
            }
            o
        }
    }

    impl Backend for Keychain {
        fn get(
            &self,
            service: &str,
            account: &str,
        ) -> Result<Option<Zeroizing<Vec<u8>>>, SshCoreError> {
            match generic_password(self.options(service, account)) {
                Ok(v) => Ok(Some(Zeroizing::new(v))),
                Err(e) if e.code() == NOT_FOUND => Ok(None),
                Err(e) => Err(err("read", &e)),
            }
        }
        fn put(
            &self,
            service: &str,
            account: &str,
            data: &[u8],
            guarded: bool,
        ) -> Result<(), SshCoreError> {
            let _ = delete_generic_password_options(self.options(service, account));
            let mut o = self.options(service, account);
            o.set_access_synchronized(Some(false));
            let ac = if guarded {
                SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleWhenPasscodeSetThisDeviceOnly),
                    AccessControlOptions::USER_PRESENCE.bits(),
                )
            } else {
                SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
                    0,
                )
            }
            .map_err(|e| err("access control", &e))?;
            o.set_access_control(ac);
            set_generic_password_options(data, o).map_err(|e| err("write", &e))
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), SshCoreError> {
            match delete_generic_password_options(self.options(service, account)) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == NOT_FOUND => Ok(()),
                Err(e) => Err(err("delete", &e)),
            }
        }
        fn delete_service(&self, service: &str) -> Result<(), SshCoreError> {
            let query = CFDictionary::from_CFType_pairs(&[
                (
                    unsafe { CFString::wrap_under_get_rule(kSecClass) }.as_CFType(),
                    unsafe { CFString::wrap_under_get_rule(kSecClassGenericPassword) }.as_CFType(),
                ),
                (
                    unsafe { CFString::wrap_under_get_rule(kSecAttrService) }.as_CFType(),
                    CFString::new(service).as_CFType(),
                ),
            ]);
            // SAFETY: `query` is a valid dictionary for the duration of the call.
            let status = unsafe { SecItemDelete(query.as_concrete_TypeRef()) };
            if status == 0 || status == NOT_FOUND {
                Ok(())
            } else {
                Err(SshCoreError::IoError {
                    reason: format!("Keychain delete failed (code {status})"),
                })
            }
        }
    }
}

fn backend_slot() -> &'static RwLock<Arc<dyn Backend>> {
    static SLOT: OnceLock<RwLock<Arc<dyn Backend>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(default_backend(None)))
}

#[cfg(all(target_vendor = "apple", not(feature = "test-support")))]
fn default_backend(group: Option<String>) -> Arc<dyn Backend> {
    Arc::new(keychain::Keychain { group })
}

#[cfg(not(all(target_vendor = "apple", not(feature = "test-support"))))]
fn default_backend(_group: Option<String>) -> Arc<dyn Backend> {
    Arc::new(Memory::default())
}

fn backend() -> Arc<dyn Backend> {
    backend_slot()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn invalid(reason: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

fn account(id: &Option<String>) -> Result<String, SshCoreError> {
    match id {
        None => Ok("ed25519_user_key".to_string()),
        Some(i)
            if !i.is_empty()
                && i.len() <= 64
                && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
        {
            Ok(format!("ed25519_key_{i}"))
        }
        Some(_) => Err(invalid("Invalid key id")),
    }
}

fn service_of(key: &KeyRef) -> &'static str {
    if key.guarded {
        GUARDED_SERVICE
    } else {
        KEY_SERVICE
    }
}

fn utf8(bytes: &[u8]) -> Result<Zeroizing<String>, SshCoreError> {
    std::str::from_utf8(bytes)
        .map(|s| Zeroizing::new(s.to_string()))
        .map_err(|_| SshCoreError::KeyParseError {
            reason: "The key is not text".to_string(),
        })
}

/// The stored key text, for the core's own use (connect, sign). Never returned over the FFI.
pub(crate) fn load_pem(key: &KeyRef) -> Result<Zeroizing<String>, SshCoreError> {
    let bytes = backend()
        .get(service_of(key), &account(&key.id)?)?
        .ok_or_else(|| SshCoreError::KeyParseError {
            reason: "No key is stored for this server".to_string(),
        })?;
    utf8(&bytes)
}

fn info_of(key: &ssh_key::PrivateKey) -> Result<KeyInfo, SshCoreError> {
    let public = key.public_key();
    Ok(KeyInfo {
        public_key_openssh: public
            .to_openssh()
            .map_err(|e| SshCoreError::KeyParseError {
                reason: format!("Failed to encode the public key: {e}"),
            })?,
        fingerprint_sha256: public.fingerprint(HashAlg::Sha256).to_string(),
    })
}

fn store(key: &KeyRef, pem: &str, overwrite: bool) -> Result<(), SshCoreError> {
    let acct = account(&key.id)?;
    let b = backend();
    if !overwrite && b.get(KEY_SERVICE, &acct)?.is_some() {
        return Err(invalid(
            "An SSH key already exists here; overwriting is protected",
        ));
    }
    b.put(KEY_SERVICE, &acct, pem.as_bytes(), false)
}

/// Whether a key is stored.
#[uniffi::export]
pub fn key_exists(key: KeyRef) -> bool {
    account(&key.id)
        .ok()
        .and_then(|a| backend().get(service_of(&key), &a).ok().flatten())
        .is_some()
}

/// The public half of a stored key, derived on demand (nothing public is stored separately).
#[uniffi::export]
pub fn key_info(key: KeyRef) -> Result<Option<KeyInfo>, SshCoreError> {
    if !key_exists(key.clone()) {
        return Ok(None);
    }
    let pem = load_pem(&key)?;
    info_of(&parse_private_key(&pem)?).map(Some)
}

/// Makes a new Ed25519 key inside the core and stores it.
#[uniffi::export]
pub fn key_generate(key: KeyRef, overwrite: bool) -> Result<KeyInfo, SshCoreError> {
    let pair = crate::keys::generate_keypair()?;
    let pem = Zeroizing::new(pair.private_key_openssh);
    store(&key, &pem, overwrite)?;
    info_of(&parse_private_key(&pem)?)
}

/// Whether an OpenSSH key file (bytes) needs a passphrase.
#[uniffi::export]
pub fn key_file_needs_passphrase(armored: Vec<u8>) -> Result<bool, SshCoreError> {
    let armored = Zeroizing::new(armored);
    let text = utf8(&armored)?;
    crate::keys::private_key_needs_passphrase(text.to_string())
}

/// Imports an OpenSSH Ed25519 key file given as bytes, decrypting it with `passphrase` when it has
/// one; the key is stored without the passphrase and neither is kept. Both inputs are wiped here.
#[uniffi::export]
pub fn key_import(
    key: KeyRef,
    armored: Vec<u8>,
    passphrase: Option<Vec<u8>>,
    overwrite: bool,
) -> Result<KeyInfo, SshCoreError> {
    let armored = Zeroizing::new(armored);
    let text = utf8(&armored)?;
    let imported = crate::keys::import_private_key(text.to_string(), passphrase)?;
    let pem = Zeroizing::new(imported.private_key_openssh);
    store(&key, &pem, overwrite)?;
    info_of(&parse_private_key(&pem)?)
}

/// Removes a key (and, for the default or any id, the guarded copy too).
#[uniffi::export]
pub fn key_delete(key: KeyRef) -> Result<(), SshCoreError> {
    let acct = account(&key.id)?;
    let b = backend();
    b.delete(KEY_SERVICE, &acct)?;
    b.delete(GUARDED_SERVICE, &acct)
}

/// Exports the key as bytes in the clear. Refused unless all three acknowledgements are given.
#[uniffi::export]
pub fn key_export_plain(key: KeyRef, acks: ExportAcks) -> Result<Vec<u8>, SshCoreError> {
    if !acks.understands_access
        || !acks.understands_exposure
        || acks.typed_confirmation != EXPORT_CONFIRMATION
    {
        return Err(invalid(
            "Exporting a private key needs all three confirmations",
        ));
    }
    Ok(load_pem(&key)?.as_bytes().to_vec())
}

fn seal_key(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, SshCoreError> {
    let params = Params::new(32 * 1024, 3, 1, Some(32)).map_err(|e| invalid(&e.to_string()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Argon::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|e| invalid(&e.to_string()))?;
    Ok(out)
}

/// The key encrypted with the owner's password (Argon2id + ChaCha20-Poly1305), for backup and
/// sync. The result is safe to store anywhere; the password is wiped.
#[uniffi::export]
pub fn key_seal(key: KeyRef, password: Vec<u8>) -> Result<Vec<u8>, SshCoreError> {
    let password = Zeroizing::new(password);
    if password.len() < MIN_SEAL_PASSWORD {
        return Err(invalid(
            "The password for an exported key must be at least 12 bytes",
        ));
    }
    let pem = load_pem(&key)?;
    let mut rng = rand::rng();
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rng.fill(&mut salt);
    rng.fill(&mut nonce);
    let k = seal_key(&password, &salt)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(k.as_ref()));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), pem.as_bytes())
        .map_err(|_| invalid("Encryption failed"))?;
    let mut out = Vec::with_capacity(SEAL_MAGIC.len() + 28 + ct.len());
    out.extend_from_slice(SEAL_MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts what [`key_seal`] made and stores the key.
#[uniffi::export]
pub fn key_unseal(
    key: KeyRef,
    sealed: Vec<u8>,
    password: Vec<u8>,
    overwrite: bool,
) -> Result<KeyInfo, SshCoreError> {
    let password = Zeroizing::new(password);
    let head = SEAL_MAGIC.len();
    if sealed.len() < head + 28 + 16 || &sealed[..head] != SEAL_MAGIC {
        return Err(invalid("Not an exported key"));
    }
    let salt = &sealed[head..head + 16];
    let nonce = &sealed[head + 16..head + 28];
    let k = seal_key(&password, salt)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(k.as_ref()));
    let plain = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(nonce), &sealed[head + 28..])
            .map_err(|_| SshCoreError::KeyPassphraseWrong)?,
    );
    let pem = utf8(&plain)?;
    let parsed = parse_private_key(&pem)?;
    if parsed.algorithm() != Algorithm::Ed25519 {
        return Err(invalid("Only Ed25519 keys are supported"));
    }
    store(&key, &pem, overwrite)?;
    info_of(&parsed)
}

/// The first stable form: the nonce came from the fingerprint alone. Still read, never written.
const STABLE_MAGIC: &[u8; 5] = b"SSHK2";
/// The stable form written now: the nonce also depends on the key text, so two different texts of
/// the same key (a re-import with another comment or check value) can never share a nonce.
const STABLE_MAGIC_V3: &[u8; 5] = b"SSHK3";

/// Derives the cipher key and nonce for the stable form from a 32-byte secret, the key's own
/// fingerprint and, for the current form, a digest of the key text. The same key text under the
/// same secret always seals to the same bytes, so sync does not see a change on every pass.
fn stable_material(
    secret: &[u8],
    fingerprint: &str,
    text_digest: Option<&[u8]>,
) -> Result<(Zeroizing<[u8; 32]>, [u8; 12]), SshCoreError> {
    use hkdf::Hkdf;
    use sha2::Sha256;
    if secret.len() != 32 {
        return Err(invalid("The sync secret must be 32 bytes"));
    }
    let hk = Hkdf::<Sha256>::new(Some(fingerprint.as_bytes()), secret);
    let mut key = Zeroizing::new([0u8; 32]);
    let mut nonce = [0u8; 12];
    match text_digest {
        None => {
            hk.expand(b"ssh-app key seal v2 key", key.as_mut())
                .map_err(|_| invalid("Key derivation failed"))?;
            hk.expand(b"ssh-app key seal v2 nonce", &mut nonce)
                .map_err(|_| invalid("Key derivation failed"))?;
        }
        Some(d) => {
            hk.expand(b"ssh-app key seal v3 key", key.as_mut())
                .map_err(|_| invalid("Key derivation failed"))?;
            let mut info = b"ssh-app key seal v3 nonce".to_vec();
            info.extend_from_slice(d);
            hk.expand(&info, &mut nonce)
                .map_err(|_| invalid("Key derivation failed"))?;
        }
    }
    Ok((key, nonce))
}

/// Like [`key_seal`] for a high-entropy 32-byte `secret` (the sync vault key) instead of a typed
/// password: HKDF instead of Argon2, and the output is stable for an unchanged key.
#[uniffi::export]
pub fn key_seal_stable(key: KeyRef, secret: Vec<u8>) -> Result<Vec<u8>, SshCoreError> {
    use sha2::{Digest, Sha256};
    let secret = Zeroizing::new(secret);
    let pem = load_pem(&key)?;
    let fp = info_of(&parse_private_key(&pem)?)?.fingerprint_sha256;
    let digest = Sha256::digest(pem.as_bytes());
    let (k, nonce) = stable_material(&secret, &fp, Some(&digest))?;
    let ct = ChaCha20Poly1305::new(Key::from_slice(k.as_ref()))
        .encrypt(Nonce::from_slice(&nonce), pem.as_bytes())
        .map_err(|_| invalid("Encryption failed"))?;
    let mut out = Vec::with_capacity(STABLE_MAGIC_V3.len() + 1 + fp.len() + 12 + ct.len());
    out.extend_from_slice(STABLE_MAGIC_V3);
    out.push(u8::try_from(fp.len()).map_err(|_| invalid("Bad fingerprint"))?);
    out.extend_from_slice(fp.as_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts what [`key_seal_stable`] made (either form) and stores the key. The stored key's
/// fingerprint must equal the one the blob names.
#[uniffi::export]
pub fn key_unseal_stable(
    key: KeyRef,
    sealed: Vec<u8>,
    secret: Vec<u8>,
    overwrite: bool,
) -> Result<KeyInfo, SshCoreError> {
    let secret = Zeroizing::new(secret);
    let head = STABLE_MAGIC.len();
    if sealed.len() < head + 1 + 16 {
        return Err(invalid("Not an exported key"));
    }
    let v3 = &sealed[..head] == STABLE_MAGIC_V3;
    if !v3 && &sealed[..head] != STABLE_MAGIC {
        return Err(invalid("Not an exported key"));
    }
    let fl = usize::from(sealed[head]);
    let nonce_len = if v3 { 12 } else { 0 };
    if sealed.len() < head + 1 + fl + nonce_len + 16 {
        return Err(invalid("Not an exported key"));
    }
    let fp = std::str::from_utf8(&sealed[head + 1..head + 1 + fl])
        .map_err(|_| invalid("Not an exported key"))?;
    let body = head + 1 + fl;
    let (k, derived) = stable_material(&secret, fp, v3.then_some(&[][..]))?;
    let nonce = if v3 {
        let mut n = [0u8; 12];
        n.copy_from_slice(&sealed[body..body + 12]);
        n
    } else {
        derived
    };
    let plain = Zeroizing::new(
        ChaCha20Poly1305::new(Key::from_slice(k.as_ref()))
            .decrypt(Nonce::from_slice(&nonce), &sealed[body + nonce_len..])
            .map_err(|_| SshCoreError::KeyPassphraseWrong)?,
    );
    let pem = utf8(&plain)?;
    let parsed = parse_private_key(&pem)?;
    let info = info_of(&parsed)?;
    if parsed.algorithm() != Algorithm::Ed25519 || info.fingerprint_sha256 != fp {
        return Err(invalid("The key does not match its label"));
    }
    store(&key, &pem, overwrite)?;
    Ok(info)
}

/// Rebuilds the guarded copies the Files extension reads (always from none, so a key no longer in
/// use leaves nothing behind). With `enabled == false` it only removes them.
#[uniffi::export]
pub fn key_publish_guarded(ids: Vec<Option<String>>, enabled: bool) -> Result<(), SshCoreError> {
    let b = backend();
    b.delete_service(GUARDED_SERVICE)?;
    if !enabled {
        return Ok(());
    }
    for id in ids {
        let acct = account(&id)?;
        if let Some(v) = b.get(KEY_SERVICE, &acct)? {
            b.put(GUARDED_SERVICE, &acct, &v, true)?;
        }
    }
    Ok(())
}

/// Sets the Keychain access group shared with the Files extension (`None` = the app's own).
#[uniffi::export]
pub fn key_store_configure(access_group: Option<String>) {
    #[cfg(all(target_vendor = "apple", not(feature = "test-support")))]
    {
        *backend_slot()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = default_backend(access_group);
    }
    #[cfg(not(all(target_vendor = "apple", not(feature = "test-support"))))]
    let _ = access_group;
}

/// For tests: stores `pem` under a fresh id and returns its reference.
#[cfg(any(test, feature = "test-support"))]
pub fn test_put(pem: &str) -> KeyRef {
    let id = format!("t{:016x}", rand::rng().random::<u64>());
    let key = KeyRef {
        id: Some(id),
        guarded: false,
    };
    if let Ok(a) = account(&key.id) {
        let _ = backend().put(KEY_SERVICE, &a, pem.as_bytes(), false);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ref() -> KeyRef {
        KeyRef {
            id: Some(format!("t{:016x}", rand::rng().random::<u64>())),
            guarded: false,
        }
    }

    #[test]
    fn generate_stores_and_reports_public_half_only() {
        let k = new_ref();
        assert!(!key_exists(k.clone()));
        let info = key_generate(k.clone(), false).unwrap();
        assert!(info.public_key_openssh.starts_with("ssh-ed25519 "));
        assert!(info.fingerprint_sha256.starts_with("SHA256:"));
        assert!(key_exists(k.clone()));
        assert_eq!(key_info(k.clone()).unwrap().unwrap(), info);
        // Never overwritten by accident.
        assert!(key_generate(k.clone(), false).is_err());
        let again = key_generate(k.clone(), true).unwrap();
        assert_ne!(again.fingerprint_sha256, info.fingerprint_sha256);
        key_delete(k.clone()).unwrap();
        assert!(!key_exists(k.clone()));
        assert!(key_info(k).unwrap().is_none());
    }

    #[test]
    fn hostile_ids_are_refused() {
        for id in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            let k = KeyRef {
                id: Some(id.to_string()),
                guarded: false,
            };
            assert!(!key_exists(k.clone()), "{id}");
            assert!(key_generate(k, false).is_err(), "{id}");
        }
    }

    #[test]
    fn import_takes_bytes_and_a_passphrase() {
        let pair = crate::keys::generate_keypair().unwrap();
        let k = new_ref();
        assert!(!key_file_needs_passphrase(pair.private_key_openssh.clone().into_bytes()).unwrap());
        let info = key_import(
            k.clone(),
            pair.private_key_openssh.clone().into_bytes(),
            None,
            false,
        )
        .unwrap();
        assert_eq!(info.public_key_openssh, pair.public_key_openssh);
        assert!(key_import(k, b"garbage".to_vec(), None, true).is_err());
    }

    #[test]
    fn plain_export_needs_all_three_confirmations() {
        let k = new_ref();
        key_generate(k.clone(), false).unwrap();
        let ok = ExportAcks {
            understands_access: true,
            understands_exposure: true,
            typed_confirmation: EXPORT_CONFIRMATION.to_string(),
        };
        for bad in [
            ExportAcks {
                understands_access: false,
                ..ok.clone()
            },
            ExportAcks {
                understands_exposure: false,
                ..ok.clone()
            },
            ExportAcks {
                typed_confirmation: "export".to_string(),
                ..ok.clone()
            },
            ExportAcks {
                typed_confirmation: String::new(),
                ..ok.clone()
            },
        ] {
            assert!(key_export_plain(k.clone(), bad).is_err());
        }
        let bytes = key_export_plain(k, ok).unwrap();
        assert!(bytes.starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----"));
    }

    #[test]
    fn sealed_key_round_trips_and_resists_tampering() {
        let src = new_ref();
        let info = key_generate(src.clone(), false).unwrap();
        let sealed = key_seal(src, b"correct horse".to_vec()).unwrap();
        assert!(
            !sealed.windows(10).any(|w| w == b"OPENSSH PR"),
            "ciphertext only"
        );
        let dst = new_ref();
        assert!(matches!(
            key_unseal(
                dst.clone(),
                sealed.clone(),
                b"wrong password".to_vec(),
                false
            ),
            Err(SshCoreError::KeyPassphraseWrong)
        ));
        assert!(!key_exists(dst.clone()));
        let mut bad = sealed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(key_unseal(dst.clone(), bad, b"correct horse".to_vec(), false).is_err());
        assert!(key_unseal(
            dst.clone(),
            b"SSHK1short".to_vec(),
            b"correct horse".to_vec(),
            false
        )
        .is_err());
        let got = key_unseal(dst, sealed, b"correct horse".to_vec(), false).unwrap();
        assert_eq!(got, info);
        assert!(key_seal(new_ref(), b"short".to_vec()).is_err());
        assert!(key_seal(new_ref(), b"elevenbytes".to_vec()).is_err());
    }

    #[test]
    fn two_texts_of_one_key_never_share_a_nonce_and_old_blobs_still_open() {
        use chacha20poly1305::aead::{Aead, KeyInit};
        let src = new_ref();
        let info = key_generate(src.clone(), false).unwrap();
        let secret = vec![9u8; 32];
        let blob = key_seal_stable(src.clone(), secret.clone()).unwrap();
        // the same key with another text (a comment added) seals under another nonce
        let pem = load_pem(&src).unwrap();
        let other_text = test_put(&format!("{}\n\n", pem.trim_end()));
        let blob2 = key_seal_stable(other_text, secret.clone()).unwrap();
        let nonce_at = |b: &[u8]| {
            let fl = usize::from(b[5]);
            b[6 + fl..6 + fl + 12].to_vec()
        };
        assert_ne!(nonce_at(&blob), nonce_at(&blob2));
        // a blob in the first form (nonce from the fingerprint alone) is still read
        let (k, nonce) = stable_material(&secret, &info.fingerprint_sha256, None).unwrap();
        let ct = ChaCha20Poly1305::new(Key::from_slice(k.as_ref()))
            .encrypt(Nonce::from_slice(&nonce), pem.as_bytes())
            .unwrap();
        let mut old = STABLE_MAGIC.to_vec();
        old.push(u8::try_from(info.fingerprint_sha256.len()).unwrap());
        old.extend_from_slice(info.fingerprint_sha256.as_bytes());
        old.extend_from_slice(&ct);
        assert_eq!(
            key_unseal_stable(new_ref(), old, secret, false).unwrap(),
            info
        );
    }

    #[test]
    fn stable_seal_is_repeatable_and_bound_to_the_secret() {
        let src = new_ref();
        let info = key_generate(src.clone(), false).unwrap();
        let secret = vec![7u8; 32];
        let a = key_seal_stable(src.clone(), secret.clone()).unwrap();
        assert_eq!(a, key_seal_stable(src.clone(), secret.clone()).unwrap());
        let other = new_ref();
        key_generate(other.clone(), false).unwrap();
        assert_ne!(a, key_seal_stable(other, secret.clone()).unwrap());
        let dst = new_ref();
        assert!(key_unseal_stable(dst.clone(), a.clone(), vec![8u8; 32], false).is_err());
        assert!(key_unseal_stable(dst.clone(), a.clone(), vec![7u8; 5], false).is_err());
        assert!(!key_exists(dst.clone()));
        let mut bad = a.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(key_unseal_stable(dst.clone(), bad, secret.clone(), false).is_err());
        assert_eq!(key_unseal_stable(dst, a, secret, false).unwrap(), info);
    }

    #[test]
    fn guarded_copies_follow_the_switch() {
        let k = new_ref();
        key_generate(k.clone(), false).unwrap();
        let g = KeyRef {
            guarded: true,
            ..k.clone()
        };
        assert!(!key_exists(g.clone()));
        key_publish_guarded(vec![k.id.clone()], true).unwrap();
        assert!(key_exists(g.clone()));
        key_publish_guarded(vec![], true).unwrap();
        assert!(!key_exists(g.clone()));
        key_publish_guarded(vec![k.id.clone()], true).unwrap();
        key_publish_guarded(vec![k.id.clone()], false).unwrap();
        assert!(!key_exists(g));
        assert!(key_exists(k));
    }

    #[test]
    fn connecting_without_a_stored_key_fails_cleanly() {
        let err = load_pem(&new_ref()).unwrap_err();
        assert!(matches!(err, SshCoreError::KeyParseError { .. }));
    }
}
