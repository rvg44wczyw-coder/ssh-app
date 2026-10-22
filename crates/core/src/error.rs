#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SshCoreError {
    #[error("Failed to generate Ed25519 keypair: {reason}")]
    KeyGenerationError { reason: String },

    #[error("Failed to parse private key: {reason}")]
    KeyParseError { reason: String },

    /// The key file is encrypted and no passphrase was given.
    #[error("The key is protected by a passphrase")]
    KeyPassphraseRequired,

    /// The passphrase did not decrypt the key.
    #[error("The passphrase does not match the key")]
    KeyPassphraseWrong,

    #[error("Invalid session name '{name}': {reason}")]
    InvalidSessionName { name: String, reason: String },

    #[error("SSH connection error: {reason}")]
    ConnectionError { reason: String },

    #[error("The host key of {host} changed (pinned {expected}, server presented {actual}); refusing to connect")]
    HostKeyChanged {
        host: String,
        expected: String,
        actual: String,
    },

    #[error("SSH authentication failed for user '{username}': {reason}")]
    AuthenticationError { username: String, reason: String },

    #[error("SSH channel error: {reason}")]
    ChannelError { reason: String },

    #[error("IO error: {reason}")]
    IoError { reason: String },

    /// The caller's input was refused before anything ran (bad rule, path, name, preset, ...).
    #[error("{reason}")]
    InvalidInput { reason: String },

    #[error("Session is already connected")]
    AlreadyConnected,

    #[error("Session is not connected")]
    NotConnected,

    #[error("Operation cancelled or timed out")]
    Timeout,

    #[error("Approval signature verification failed: {reason}")]
    ApprovalVerificationError { reason: String },

    #[error("Invalid approval payload: {reason}")]
    InvalidApprovalPayload { reason: String },

    #[error("Port forward failed: {reason}")]
    PortForwardFailed { reason: String },

    #[error("Local port {port} is already bound")]
    LocalPortInUse { port: u16 },

    #[error("Remote host refused direct-tcpip channel for port {port}")]
    RemotePortRefused { port: u16 },

    #[error("AI assistant error: {reason}")]
    AiAssistantError { reason: String },

    #[error("Failed to parse AI response: {reason}")]
    AiResponseParseError { reason: String },

    #[error("Transcript error: {reason}")]
    TranscriptError { reason: String },

    #[error("Transcript is larger than the safety limit ({bytes} bytes)")]
    TranscriptTooLarge { bytes: u64 },

    #[error("Invalid transcript path '{path}': {reason}")]
    InvalidTranscriptPath { path: String, reason: String },

    #[error("No such file or directory: {path}")]
    FileNotFound { path: String },

    #[error("Not a regular file: {path}")]
    NotARegularFile { path: String },

    #[error("Already exists: {path}")]
    AlreadyExists { path: String },

    #[error("File is larger than the allowed {limit} bytes")]
    TooLarge { limit: u64 },

    #[error("Transfer cancelled")]
    Cancelled,

    #[error("Not available: {reason}")]
    Unsupported { reason: String },
}
