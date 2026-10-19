#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_pem: String,
    pub session_name: String,
    pub initial_cols: u16,
    pub initial_rows: u16,
    pub command: Option<String>,
    /// Folder a **new** tmux session starts in (created if missing; `~/` is the home folder).
    /// Ignored when the session already exists. `None` = the login shell's default (home).
    pub start_dir: Option<String>,
    /// Pinned host key fingerprint (`SHA256:...`). `None` accepts the first key seen (see
    /// `SshSessionHandle::host_key_fingerprint`); `Some` refuses any other key.
    pub pinned_host_key: Option<String>,
    /// What keeps the session alive on the host: tmux, or the helper's `mux` on native Windows.
    pub multiplexer: crate::multiplexer::MultiplexerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_pem: String,
    pub pinned_host_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SessionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Failed { reason: String },
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}
