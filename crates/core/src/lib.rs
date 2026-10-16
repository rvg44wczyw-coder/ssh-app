uniffi::setup_scaffolding!();

pub mod ai;
pub mod approval;
pub mod buffer;
pub mod config;
pub mod error;
pub mod file_browser;
pub mod keys;
pub mod port_forward;
pub mod session;
pub mod snippets;
pub mod telemetry;
pub mod tmux;
pub mod transcript;

pub use ai::{
    assemble_ai_system_prompt, assemble_ai_user_prompt, classify_command_risk, parse_ai_response,
    parse_ollama_generate_response, sanitize_shell_command, AiCommandRequest, AiCommandSuggestion,
    AiRiskAssessment, AiRiskLevel,
};
pub use approval::{
    create_canonical_signing_bytes, hash_command_sha256, verify_approval_freshness,
    verify_approval_signature, CanonicalSigningPayload,
};
pub use config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
pub use error::{SshCoreError, TranscriptError};
pub use file_browser::{
    parse_remote_directory_listing, quote_remote_path_for_shell, RemoteDirectoryListing,
    RemoteFileEntry, RemoteFileKind, RemoteFilePreview,
};
pub use keys::{derive_public_key, generate_keypair, KeypairResult};
pub use port_forward::{PortForwardHandle, PortForwardInfo};
pub use session::{
    execute_remote_command, kill_remote_tmux_session, list_remote_tmux_sessions,
    SshSessionCallback, SshSessionHandle,
};
pub use snippets::{
    default_developer_snippets, extract_history_commands, extract_snippet_placeholders,
    fuzzy_search_snippets, FuzzyMatchResult, SnippetItem,
};
pub use telemetry::{
    classify_process_category, parse_host_telemetry, HostTelemetrySnapshot, ProcessCategory,
    RemoteProcessInfo,
};
pub use tmux::TmuxSessionInfo;
pub use transcript::{
    parse_transcript_jsonl, strip_ansi_codes, TranscriptStatus, TranscriptToolCall, TranscriptTurn,
    TranscriptTurnType,
};

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
