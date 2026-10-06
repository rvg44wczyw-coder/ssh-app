use crate::error::SshCoreError;
use ssh_key::{Algorithm, LineEnding, PrivateKey};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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

#[uniffi::export]
pub fn derive_public_key(private_key_openssh: String) -> Result<String, SshCoreError> {
    let priv_key = parse_private_key(&private_key_openssh)?;
    priv_key
        .public_key()
        .to_openssh()
        .map_err(|e| SshCoreError::KeyParseError {
            reason: format!("Failed to encode public key to OpenSSH format: {e}"),
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
}
