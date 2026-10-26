use crate::error::SshCoreError;
use ssh_key::{Algorithm, HashAlg, Kdf, LineEnding, PrivateKey};
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeypairResult {
    pub public_key_openssh: String,
    pub private_key_openssh: String,
}

pub fn generate_keypair() -> Result<KeypairResult, SshCoreError> {
    let mut rng = rand::rng();
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(|e| {
        SshCoreError::KeyGenerationError {
            reason: format!("Failed to generate Ed25519 key: {e}"),
        }
    })?;

    let private_key_openssh = private_key
        .to_openssh(LineEnding::LF)
        .map_err(|e| SshCoreError::KeyGenerationError {
            reason: format!("Failed to encode private key to OpenSSH format: {e}"),
        })?
        .to_string();

    let public_key_openssh =
        private_key
            .public_key()
            .to_openssh()
            .map_err(|e| SshCoreError::KeyGenerationError {
                reason: format!("Failed to encode public key to OpenSSH format: {e}"),
            })?;

    Ok(KeypairResult {
        public_key_openssh,
        private_key_openssh,
    })
}

pub fn parse_private_key(pem_or_openssh: &str) -> Result<PrivateKey, SshCoreError> {
    PrivateKey::from_openssh(pem_or_openssh.trim()).map_err(|e| SshCoreError::KeyParseError {
        reason: format!("Invalid OpenSSH private key: {e}"),
    })
}

pub fn derive_public_key(private_key_openssh: String) -> Result<String, SshCoreError> {
    let priv_key = parse_private_key(&private_key_openssh)?;
    priv_key
        .public_key()
        .to_openssh()
        .map_err(|e| SshCoreError::KeyParseError {
            reason: format!("Failed to encode public key to OpenSSH format: {e}"),
        })
}

/// A bcrypt-pbkdf rounds count above this in a key file is refused: the count comes from the file,
/// and a hostile one would hold the phone for minutes. `ssh-keygen` defaults to 16.
pub const MAX_BCRYPT_ROUNDS: u32 = 500;

/// What `import_private_key` hands back: the key in the unencrypted form the Keychain stores (the
/// Keychain item is the protection) and what the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedKey {
    pub private_key_openssh: String,
    pub public_key_openssh: String,
    pub fingerprint_sha256: String,
    pub was_encrypted: bool,
}

/// Whether the armored key needs a passphrase, without trying to decrypt it.
pub fn private_key_needs_passphrase(armored_key: String) -> Result<bool, SshCoreError> {
    Ok(parse_private_key(&armored_key)?.is_encrypted())
}

/// Imports an OpenSSH private key, decrypting it with `passphrase` when it is protected. Only
/// Ed25519 is accepted: that is what the connection path signs with. The passphrase is wiped when
/// this returns.
pub fn import_private_key(
    armored_key: String,
    passphrase: Option<Vec<u8>>,
) -> Result<ImportedKey, SshCoreError> {
    import_private_key_limited(armored_key, passphrase, MAX_BCRYPT_ROUNDS)
}

fn import_private_key_limited(
    armored_key: String,
    passphrase: Option<Vec<u8>>,
    max_rounds: u32,
) -> Result<ImportedKey, SshCoreError> {
    let passphrase = passphrase.map(Zeroizing::new);
    let armored = Zeroizing::new(armored_key);
    let parsed = parse_private_key(&armored)?;
    let was_encrypted = parsed.is_encrypted();
    let key = if was_encrypted {
        if let Kdf::Bcrypt { rounds, .. } = parsed.kdf() {
            if *rounds > max_rounds {
                return Err(SshCoreError::KeyParseError {
                    reason: format!(
                        "The key asks for {rounds} bcrypt rounds; more than {max_rounds} is refused"
                    ),
                });
            }
        }
        let pass = passphrase
            .as_ref()
            .filter(|p| !p.is_empty())
            .ok_or(SshCoreError::KeyPassphraseRequired)?;
        parsed
            .decrypt(pass.as_slice())
            .map_err(|_| SshCoreError::KeyPassphraseWrong)?
    } else {
        parsed
    };
    if key.algorithm() != Algorithm::Ed25519 {
        return Err(SshCoreError::KeyParseError {
            reason: format!(
                "{} keys are not supported yet; only Ed25519 (ssh-keygen -t ed25519)",
                key.algorithm().as_str()
            ),
        });
    }
    let private_key_openssh = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| SshCoreError::KeyParseError {
            reason: format!("Failed to encode the private key: {e}"),
        })?
        .to_string();
    let public = key.public_key();
    Ok(ImportedKey {
        private_key_openssh,
        public_key_openssh: public
            .to_openssh()
            .map_err(|e| SshCoreError::KeyParseError {
                reason: format!("Failed to encode the public key: {e}"),
            })?,
        fingerprint_sha256: public.fingerprint(HashAlg::Sha256).to_string(),
        was_encrypted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_parse_keypair() {
        let keys = generate_keypair().expect("Key generation should succeed");
        assert!(keys.public_key_openssh.starts_with("ssh-ed25519 "));
        assert!(keys
            .private_key_openssh
            .contains("BEGIN OPENSSH PRIVATE KEY"));

        let parsed = parse_private_key(&keys.private_key_openssh);
        assert!(parsed.is_ok(), "Private key should parse back successfully");

        let derived =
            derive_public_key(keys.private_key_openssh.clone()).expect("Derivation succeeds");
        assert_eq!(derived.trim(), keys.public_key_openssh.trim());
    }

    #[test]
    fn test_parse_invalid_key() {
        let result = parse_private_key("not a valid ssh key");
        assert!(result.is_err());
        match result {
            Err(SshCoreError::KeyParseError { reason }) => {
                assert!(!reason.is_empty());
            }
            _ => panic!("Expected KeyParseError"),
        }
    }

    fn encrypted(pass: &str) -> (String, String) {
        let pair = generate_keypair().unwrap();
        let key = parse_private_key(&pair.private_key_openssh).unwrap();
        let enc = key.encrypt(&mut rand::rng(), pass).unwrap();
        (
            enc.to_openssh(LineEnding::LF).unwrap().to_string(),
            pair.public_key_openssh,
        )
    }

    #[test]
    fn imports_an_unencrypted_key_and_reports_its_fingerprint() {
        let pair = generate_keypair().unwrap();
        let got = import_private_key(pair.private_key_openssh.clone(), None).unwrap();
        assert!(!got.was_encrypted);
        assert_eq!(
            got.public_key_openssh.trim(),
            pair.public_key_openssh.trim()
        );
        assert!(got.fingerprint_sha256.starts_with("SHA256:"));
        assert!(!private_key_needs_passphrase(pair.private_key_openssh).unwrap());
    }

    #[test]
    fn imports_a_passphrase_key_and_stores_it_unencrypted() {
        let (armored, public) = encrypted("correct horse");
        assert!(private_key_needs_passphrase(armored.clone()).unwrap());
        let got = import_private_key(armored, Some(b"correct horse".to_vec())).unwrap();
        assert!(got.was_encrypted);
        assert_eq!(got.public_key_openssh.trim(), public.trim());
        assert!(!parse_private_key(&got.private_key_openssh)
            .unwrap()
            .is_encrypted());
    }

    #[test]
    fn a_wrong_or_missing_passphrase_is_a_typed_error() {
        let (armored, _) = encrypted("right");
        assert_eq!(
            import_private_key(armored.clone(), Some(b"wrong".to_vec())).unwrap_err(),
            SshCoreError::KeyPassphraseWrong
        );
        assert_eq!(
            import_private_key(armored.clone(), None).unwrap_err(),
            SshCoreError::KeyPassphraseRequired
        );
        assert_eq!(
            import_private_key(armored, Some(Vec::new())).unwrap_err(),
            SshCoreError::KeyPassphraseRequired
        );
    }

    #[test]
    fn a_rounds_count_above_the_ceiling_is_refused_before_deriving() {
        let (armored, _) = encrypted("pw");
        let err = import_private_key_limited(armored, Some(b"pw".to_vec()), 8).unwrap_err();
        assert!(matches!(err, SshCoreError::KeyParseError { reason } if reason.contains("rounds")));
    }

    #[test]
    fn other_key_types_are_refused_with_a_reason() {
        let key = PrivateKey::random(
            &mut rand::rng(),
            Algorithm::Ecdsa {
                curve: ssh_key::EcdsaCurve::NistP256,
            },
        )
        .unwrap();
        let armored = key.to_openssh(LineEnding::LF).unwrap().to_string();
        let err = import_private_key(armored, None).unwrap_err();
        assert!(
            matches!(err, SshCoreError::KeyParseError { reason } if reason.contains("Ed25519"))
        );
    }

    #[test]
    fn garbage_is_a_parse_error_not_a_panic() {
        assert!(matches!(
            import_private_key("not a key".into(), None),
            Err(SshCoreError::KeyParseError { .. })
        ));
    }

    #[test]
    fn a_file_that_asks_for_billions_of_rounds_is_refused_at_once() {
        let (armored, _) = encrypted("pw");
        let key = parse_private_key(&armored).unwrap();
        let Kdf::Bcrypt { salt, rounds } = key.kdf().clone() else {
            panic!("expected bcrypt");
        };
        let mut blob = key.to_bytes().unwrap().to_vec();
        let mut needle = salt.clone();
        needle.extend_from_slice(&rounds.to_be_bytes());
        let at = blob
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .expect("salt and rounds are in the blob");
        let rounds_at = at + salt.len();
        blob[rounds_at..rounds_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let hostile = PrivateKey::from_bytes(&blob)
            .unwrap()
            .to_openssh(LineEnding::LF)
            .unwrap()
            .to_string();
        let started = std::time::Instant::now();
        let err = import_private_key(hostile, Some(b"pw".to_vec())).unwrap_err();
        assert!(matches!(err, SshCoreError::KeyParseError { reason } if reason.contains("rounds")));
        assert!(started.elapsed().as_secs() < 2, "refused without deriving");
    }
}
