uniffi::setup_scaffolding!();

pub mod approval;
pub mod buffer;
pub mod config;
pub mod error;
pub mod keys;
pub mod session;
pub mod tmux;

pub use approval::{
    create_canonical_signing_bytes, hash_command_sha256, verify_approval_freshness,
    verify_approval_signature, CanonicalSigningPayload,
};
pub use config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
pub use error::SshCoreError;
pub use keys::{derive_public_key, generate_keypair, KeypairResult};
pub use session::{
    execute_remote_command, kill_remote_tmux_session, list_remote_tmux_sessions,
    SshSessionCallback, SshSessionHandle,
};
pub use tmux::TmuxSessionInfo;

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[uniffi::export]
pub fn generate_ssh_keypair() -> Result<KeypairResult, SshCoreError> {
    keys::generate_keypair()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_version() {
        assert_eq!(core_version(), "0.1.0");
    }

    #[test]
    fn test_generate_ssh_keypair_export() {
        let keys = generate_ssh_keypair().expect("Should generate keypair");
        assert!(keys.public_key_openssh.starts_with("ssh-ed25519 "));
    }
}
