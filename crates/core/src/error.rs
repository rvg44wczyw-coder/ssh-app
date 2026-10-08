#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SshCoreError {
    #[error("Failed to generate Ed25519 keypair: {reason}")]
    KeyGenerationError { reason: String },

    #[error("Failed to parse private key: {reason}")]
    KeyParseError { reason: String },

    #[error("Invalid session name '{name}': {reason}")]
    InvalidSessionName { name: String, reason: String },

    #[error("SSH connection error: {reason}")]
    ConnectionError { reason: String },

    #[error("SSH authentication failed for user '{username}': {reason}")]
    AuthenticationError { username: String, reason: String },

    #[error("SSH channel error: {reason}")]
    ChannelError { reason: String },

    #[error("IO error: {reason}")]
    IoError { reason: String },

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
}
