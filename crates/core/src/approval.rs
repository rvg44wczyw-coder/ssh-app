use crate::error::SshCoreError;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use ssh_key::PublicKey;

/// Canonical signing payload representation for agent approvals.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CanonicalSigningPayload {
    pub id: String,
    pub command_hash_sha256: String,
    pub nonce: String,
    pub timestamp_sec: u64,
    pub approved: bool,
}

impl CanonicalSigningPayload {
    pub fn to_bytes(&self) -> Vec<u8> {
        format!(
            "SSH_APP_APPROVAL_V1:{}:{}:{}:{}:{}",
            self.id, self.command_hash_sha256, self.nonce, self.timestamp_sec, self.approved
        )
        .into_bytes()
    }
}

/// Computes SHA256 hash of a command string formatted as lowercase hex.
#[uniffi::export]
pub fn hash_command_sha256(command: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    hex::encode(hasher.finalize())
}

/// Serializes approval data into canonical bytes for signing.
#[uniffi::export]
pub fn create_canonical_signing_bytes(
    id: String,
    command_hash_sha256: String,
    nonce: String,
    timestamp_sec: u64,
    approved: bool,
) -> Vec<u8> {
    let payload = CanonicalSigningPayload {
        id,
        command_hash_sha256,
        nonce,
        timestamp_sec,
        approved,
    };
    payload.to_bytes()
}

/// Verifies whether an approval decision's timestamp falls within the allowed tolerance window.
#[uniffi::export]
pub fn verify_approval_freshness(
    timestamp_sec: u64,
    current_time_sec: u64,
    max_drift_sec: u64,
) -> bool {
    if timestamp_sec > current_time_sec + max_drift_sec {
        return false;
    }
    current_time_sec.saturating_sub(timestamp_sec) <= max_drift_sec
}

/// Verifies an Ed25519 signature over canonical approval bytes against an OpenSSH public key.
#[uniffi::export]
pub fn verify_approval_signature(
    public_key_openssh: String,
    canonical_bytes: Vec<u8>,
    signature_hex: String,
) -> Result<bool, SshCoreError> {
    let pubkey = PublicKey::from_openssh(public_key_openssh.trim()).map_err(|e| {
        SshCoreError::KeyParseError {
            reason: format!("Failed to parse public key: {e}"),
        }
    })?;

    let sig_bytes =
        hex::decode(signature_hex.trim()).map_err(|e| SshCoreError::ApprovalVerificationError {
            reason: format!("Signature is not valid hex: {e}"),
        })?;

    if sig_bytes.len() != 64 {
        return Err(SshCoreError::ApprovalVerificationError {
            reason: format!(
                "Invalid Ed25519 signature length: expected 64, got {}",
                sig_bytes.len()
            ),
        });
    }

    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let signature = Signature::from_bytes(&sig_arr);

    match pubkey.key_data() {
        ssh_key::public::KeyData::Ed25519(ed_pubkey) => {
            let verifying_key = VerifyingKey::from_bytes(&ed_pubkey.0).map_err(|e| {
                SshCoreError::ApprovalVerificationError {
                    reason: format!("Invalid Ed25519 public key bytes: {e}"),
                }
            })?;

            verifying_key
                .verify(&canonical_bytes, &signature)
                .map_err(|e| SshCoreError::ApprovalVerificationError {
                    reason: format!("Cryptographic signature verification failed: {e}"),
                })?;
            Ok(true)
        }
        _ => Err(SshCoreError::ApprovalVerificationError {
            reason: "Only Ed25519 public keys are supported for agent approvals".to_string(),
        }),
    }
}

/// Helper for signing canonical approval bytes on client with private key.
#[uniffi::export]
pub fn sign_approval_payload(
    private_key_openssh: String,
    canonical_bytes: Vec<u8>,
) -> Result<String, SshCoreError> {
    let priv_key = crate::keys::parse_private_key(&private_key_openssh)?;

    match priv_key.key_data() {
        ssh_key::private::KeypairData::Ed25519(ed_keypair) => {
            let signing_key = SigningKey::from_bytes(ed_keypair.private.as_ref());
            let signature = signing_key.sign(&canonical_bytes);
            Ok(hex::encode(signature.to_bytes()))
        }
        _ => Err(SshCoreError::ApprovalVerificationError {
            reason: "Only Ed25519 private keys are supported for agent approvals".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::generate_keypair;

    #[test]
    fn test_canonical_bytes_deterministic() {
        let b1 = create_canonical_signing_bytes(
            "appr-1".to_string(),
            "abcd1234".to_string(),
            "nonce123".to_string(),
            1718000000,
            true,
        );
        let b2 = create_canonical_signing_bytes(
            "appr-1".to_string(),
            "abcd1234".to_string(),
            "nonce123".to_string(),
            1718000000,
            true,
        );
        assert_eq!(b1, b2);
        assert_eq!(
            String::from_utf8(b1).unwrap(),
            "SSH_APP_APPROVAL_V1:appr-1:abcd1234:nonce123:1718000000:true"
        );
    }

    #[test]
    fn test_command_hashing() {
        let hash = hash_command_sha256("git push origin main".to_string());
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            "16f880284c51ff513ff5465f0082c75d9c7ebb186e65e98b4fa362534044846a"
        );
    }

    #[test]
    fn test_freshness_verification() {
        assert!(verify_approval_freshness(1000, 1050, 120));
        assert!(verify_approval_freshness(1000, 1000, 120));
        assert!(!verify_approval_freshness(1000, 1150, 120));
        // Future timestamp beyond tolerance rejected
        assert!(!verify_approval_freshness(1150, 1000, 120));
        assert!(verify_approval_freshness(1050, 1000, 120));
    }

    #[test]
    fn test_sign_and_verify_approval_roundtrip() {
        let keypair = generate_keypair().expect("Key generation failed");
        let canonical_bytes = create_canonical_signing_bytes(
            "approval-42".to_string(),
            hash_command_sha256("cargo test".to_string()),
            "a1b2c3d4e5f6".to_string(),
            1728000000,
            true,
        );

        let signature_hex =
            sign_approval_payload(keypair.private_key_openssh.clone(), canonical_bytes.clone())
                .expect("Signing failed");
        assert_eq!(signature_hex.len(), 128); // 64 bytes = 128 hex chars

        let valid = verify_approval_signature(
            keypair.public_key_openssh.clone(),
            canonical_bytes.clone(),
            signature_hex.clone(),
        )
        .expect("Verification should succeed");
        assert!(valid);

        // Verification fails if payload is tampered
        let tampered_bytes = create_canonical_signing_bytes(
            "approval-42".to_string(),
            hash_command_sha256("rm -rf /".to_string()), // tampered command
            "a1b2c3d4e5f6".to_string(),
            1728000000,
            true,
        );
        let tampered_result =
            verify_approval_signature(keypair.public_key_openssh, tampered_bytes, signature_hex);
        assert!(tampered_result.is_err());
    }
}
