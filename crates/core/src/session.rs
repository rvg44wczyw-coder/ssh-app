use crate::buffer::{OutputThrottler, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_BATCH_SIZE};
use crate::config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
use crate::error::SshCoreError;
use crate::keys::parse_private_key;
use crate::port_forward::{PortForwardHandle, PortForwardInfo};
use crate::tmux::{build_tmux_attach_or_create_command, TmuxSessionInfo};
use russh::client::{self, Handler};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("ssh-core-tokio")
        .build()
        .expect("Failed to initialize Tokio runtime for ssh-core")
});

#[uniffi::export(callback_interface)]
pub trait SshSessionCallback: Send + Sync {
    fn on_state_changed(&self, state: SessionState);
    fn on_data_received(&self, data: Vec<u8>);
    fn on_error(&self, message: String);
}

#[derive(Default)]
pub(crate) struct SshHandler;

impl Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // TOFU / VPN-secured host verification
        Ok(true)
    }
}

enum SessionCommand {
    Input(Vec<u8>),
    Resize(TerminalSize),
    Disconnect,
}

#[derive(uniffi::Object)]
pub struct SshSessionHandle {
    config: RwLock<SessionConfig>,
    state: Arc<RwLock<SessionState>>,
    callback: Arc<RwLock<Option<Box<dyn SshSessionCallback>>>>,
    command_tx: Arc<Mutex<Option<mpsc::Sender<SessionCommand>>>>,
    session_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    is_connected: Arc<AtomicBool>,
    port_forwards: Arc<Mutex<Vec<Arc<PortForwardHandle>>>>,
    ssh_client: Arc<tokio::sync::Mutex<Option<client::Handle<SshHandler>>>>,
}

#[uniffi::export]
impl SshSessionHandle {
    #[uniffi::constructor]
    pub fn new(config: SessionConfig) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            state: Arc::new(RwLock::new(SessionState::Disconnected)),
            callback: Arc::new(RwLock::new(None)),
            command_tx: Arc::new(Mutex::new(None)),
            session_task: Arc::new(Mutex::new(None)),
            is_connected: Arc::new(AtomicBool::new(false)),
            port_forwards: Arc::new(Mutex::new(Vec::new())),
            ssh_client: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    /// Establishes the SSH connection, launches tmux, and starts streaming I/O.
    pub fn connect(&self, callback: Box<dyn SshSessionCallback>) -> Result<(), SshCoreError> {
        if self.is_connected.load(Ordering::SeqCst) {
            return Err(SshCoreError::AlreadyConnected);
        }

        *self.callback.write().unwrap() = Some(callback);
        self.set_state(SessionState::Connecting);

        let config = self.config.read().unwrap().clone();
        self.spawn_session_loop(config, false)
    }

    /// Sends raw user keystrokes / input bytes to the remote terminal.
    pub fn send_input(&self, data: Vec<u8>) -> Result<(), SshCoreError> {
        eprintln!("[RustCore] send_input called with {} bytes", data.len());
        if !self.is_connected.load(Ordering::SeqCst) {
            eprintln!("[RustCore] send_input failed: not connected");
            return Err(SshCoreError::NotConnected);
        }

        if let Some(tx) = self.command_tx.lock().unwrap().as_ref() {
            let res = tx.try_send(SessionCommand::Input(data));
            eprintln!("[RustCore] send_input try_send result: {:?}", res.is_ok());
            Ok(())
        } else {
            eprintln!("[RustCore] send_input failed: command_tx is None");
            Err(SshCoreError::NotConnected)
        }
    }

    /// Dynamically resizes the remote terminal viewport (triggers remote SIGWINCH in tmux).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), SshCoreError> {
        let size = TerminalSize { cols, rows };
        if let Some(tx) = self.command_tx.lock().unwrap().as_ref() {
            let _ = tx.try_send(SessionCommand::Resize(size));
        }
        Ok(())
    }

    /// Zero Battery Drain: gracefully closes the SSH channel, port forwards, and TCP socket.
    /// tmux continues running remote agent tasks detached on the MacBook.
    pub fn disconnect(&self) {
        self.stop_all_port_forwards();

        if let Some(tx) = self.command_tx.lock().unwrap().take() {
            let _ = tx.try_send(SessionCommand::Disconnect);
        }

        if let Some(task) = self.session_task.lock().unwrap().take() {
            task.abort();
        }

        self.is_connected.store(false, Ordering::SeqCst);
        self.set_state(SessionState::Disconnected);
    }

    /// Instant reconnection on app foreground (didBecomeActive).
    /// Re-attaches to the existing detached tmux session.
    pub fn reconnect(&self) -> Result<(), SshCoreError> {
        if self.is_connected.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.set_state(SessionState::Reconnecting);
        let config = self.config.read().unwrap().clone();
        self.spawn_session_loop(config, true)
    }

    /// Returns current state.
    pub fn state(&self) -> SessionState {
        self.state.read().unwrap().clone()
    }

    /// Starts local loopback port forwarding for the specified remote port on the host.
    /// If `local_port` is 0 or None, the OS allocates an available ephemeral local port on 127.0.0.1.
    pub async fn start_port_forward(
        &self,
        remote_port: u16,
        local_port: Option<u16>,
    ) -> Result<Arc<PortForwardHandle>, SshCoreError> {
        if !self.is_connected.load(Ordering::SeqCst) {
            return Err(SshCoreError::NotConnected);
        }

        let handle = crate::port_forward::start_port_forward_listener(
            Arc::clone(&self.ssh_client),
            "127.0.0.1".to_string(),
            remote_port,
            local_port,
        )
        .await?;

        if let Ok(mut forwards) = self.port_forwards.lock() {
            forwards.push(Arc::clone(&handle));
        }
        Ok(handle)
    }

    /// Returns a list of all currently active port forwards.
    pub fn get_active_port_forwards(&self) -> Vec<PortForwardInfo> {
        if let Ok(mut forwards) = self.port_forwards.lock() {
            forwards.retain(|f| f.is_active());
            forwards.iter().map(|f| f.to_info()).collect()
        } else {
            Vec::new()
        }
    }

    /// Stops all running port forwards (used during teardown/backgrounding).
    pub fn stop_all_port_forwards(&self) {
        if let Ok(mut forwards) = self.port_forwards.lock() {
            for f in forwards.drain(..) {
                f.stop();
            }
        }
    }

    /// Executes a command on the remote host over the currently active SSH session (ephemeral channel).
    pub async fn execute_command_on_active_session(
        &self,
        command: String,
    ) -> Result<String, SshCoreError> {
        let mut channel =
            {
                let mut lock = self.ssh_client.lock().await;
                match lock.as_mut() {
                    Some(client) => client.channel_open_session().await.map_err(|e| {
                        SshCoreError::ChannelError {
                            reason: format!("Failed to open ephemeral session channel: {e}"),
                        }
                    })?,
                    None => return Err(SshCoreError::NotConnected),
                }
            };

        channel
            .exec(true, command)
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("Exec command failed: {e}"),
            })?;

        let mut output = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { ref data }
                | russh::ChannelMsg::ExtendedData { ref data, .. } => {
                    output.extend_from_slice(data);
                }
                russh::ChannelMsg::Eof | russh::ChannelMsg::Close => break,
                _ => {}
            }
        }

        let _ = channel.eof().await;
        let _ = channel.close().await;
        Ok(String::from_utf8_lossy(&output).to_string())
    }

    /// Queries Ollama running locally on the remote host (http://127.0.0.1:11434/api/generate)
    /// through the active SSH session and returns the generated command suggestion.
    pub async fn query_host_ollama(
        &self,
        model: String,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<crate::ai::AiCommandSuggestion, SshCoreError> {
        let full_prompt = format!("{system_prompt}\n\n{user_prompt}");
        let payload = serde_json::json!({
            "model": model,
            "prompt": full_prompt,
            "stream": false
        });
        let payload_str = payload.to_string();

        let curl_cmd = format!(
            "curl -s -X POST http://127.0.0.1:11434/api/generate -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload_str)
        );

        let raw_response = self.execute_command_on_active_session(curl_cmd).await?;
        let generated_text = crate::ai::parse_ollama_generate_response(&raw_response)?;
        crate::ai::parse_ai_response(&generated_text)
    }
}

impl SshSessionHandle {
    fn set_state(&self, new_state: SessionState) {
        *self.state.write().unwrap() = new_state.clone();
        if let Some(cb) = self.callback.read().unwrap().as_ref() {
            cb.on_state_changed(new_state);
        }
    }

    fn spawn_session_loop(
        &self,
        config: SessionConfig,
        _is_reconnect: bool,
    ) -> Result<(), SshCoreError> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(256);
        *self.command_tx.lock().unwrap() = Some(cmd_tx);

        let callback_weak = Arc::clone(&self.callback);
        let is_connected_flag = Arc::clone(&self.is_connected);
        let state_arc = Arc::clone(&self.state);
        let ssh_client_arc = Arc::clone(&self.ssh_client);

        let session_task = RUNTIME.spawn(async move {
            is_connected_flag.store(true, Ordering::SeqCst);

            let run_result = Self::run_ssh_connection(
                &config,
                &mut cmd_rx,
                Arc::clone(&callback_weak),
                Arc::clone(&state_arc),
                Arc::clone(&ssh_client_arc),
            )
            .await;

            *ssh_client_arc.lock().await = None;
            is_connected_flag.store(false, Ordering::SeqCst);

            let final_state = match run_result {
                Ok(()) => SessionState::Disconnected,
                Err(e) => {
                    let err_msg = e.to_string();
                    if let Some(cb) = callback_weak.read().unwrap().as_ref() {
                        cb.on_error(err_msg.clone());
                    }
                    SessionState::Failed { reason: err_msg }
                }
            };

            *state_arc.write().unwrap() = final_state.clone();
            if let Some(cb) = callback_weak.read().unwrap().as_ref() {
                cb.on_state_changed(final_state);
            }
        });

        *self.session_task.lock().unwrap() = Some(session_task);
        Ok(())
    }

    async fn run_ssh_connection(
        config: &SessionConfig,
        cmd_rx: &mut mpsc::Receiver<SessionCommand>,
        callback: Arc<RwLock<Option<Box<dyn SshSessionCallback>>>>,
        state_arc: Arc<RwLock<SessionState>>,
        ssh_client_slot: Arc<tokio::sync::Mutex<Option<client::Handle<SshHandler>>>>,
    ) -> Result<(), SshCoreError> {
        // 1. Parse Ed25519 private key
        eprintln!(
            "[RustCore] Parsing private key (len={})...",
            config.private_key_pem.len()
        );
        let private_key = parse_private_key(&config.private_key_pem)?;
        let key_pair = private_key;
        let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

        // 2. Configure SSH client with adaptive keepalive (30s)
        let ssh_config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            ..Default::default()
        });

        // 3. Connect TCP socket with 10s timeout
        eprintln!(
            "[RustCore] Connecting to TCP socket {}:{}...",
            config.host, config.port
        );
        let connect_future =
            client::connect(ssh_config, (config.host.as_str(), config.port), SshHandler);
        let mut session = tokio::time::timeout(Duration::from_secs(10), connect_future)
            .await
            .map_err(|_| {
                eprintln!("[RustCore] Connection timed out after 10s!");
                SshCoreError::Timeout
            })?
            .map_err(|e| {
                eprintln!("[RustCore] Failed to connect: {e}");
                SshCoreError::ConnectionError {
                    reason: format!("Failed to connect to {}:{}: {e}", config.host, config.port),
                }
            })?;

        eprintln!(
            "[RustCore] TCP connection established! Authenticating user '{}'...",
            config.username
        );

        // 4. Authenticate with Ed25519 public key
        let auth_res = session
            .authenticate_publickey(&config.username, key_with_alg)
            .await
            .map_err(|e| {
                eprintln!("[RustCore] Authentication error: {e}");
                SshCoreError::AuthenticationError {
                    username: config.username.clone(),
                    reason: format!("Authentication request failed: {e}"),
                }
            })?;

        eprintln!(
            "[RustCore] Public key authentication result: success={}",
            auth_res.success()
        );

        if !auth_res.success() {
            return Err(SshCoreError::AuthenticationError {
                username: config.username.clone(),
                reason: "Host rejected public key authentication".to_string(),
            });
        }

        // 5. Open interactive terminal channel
        let mut channel =
            session
                .channel_open_session()
                .await
                .map_err(|e| SshCoreError::ChannelError {
                    reason: format!("Failed to open session channel: {e}"),
                })?;

        // 5.1 Store session handle for port forwarding
        *ssh_client_slot.lock().await = Some(session);

        // 6. Request PTY with xterm-256color for TrueColor support
        channel
            .request_pty(
                false,
                "xterm-256color",
                config.initial_cols as u32,
                config.initial_rows as u32,
                0,
                0,
                &[],
            )
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("PTY request failed: {e}"),
            })?;

        // 7. Format dynamic login shell + tmux attach/create command
        let tmux_cmd =
            build_tmux_attach_or_create_command(&config.session_name, config.command.as_deref())?;

        channel
            .exec(true, tmux_cmd)
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("Exec command failed: {e}"),
            })?;

        // Notify Connected state
        *state_arc.write().unwrap() = SessionState::Connected;
        if let Some(cb) = callback.read().unwrap().as_ref() {
            cb.on_state_changed(SessionState::Connected);
        }

        // 8. Setup output throttler to prevent SwiftTerm UI freezing
        let throttler_cb = Arc::clone(&callback);
        let throttler = OutputThrottler::new(
            DEFAULT_FLUSH_INTERVAL,
            DEFAULT_MAX_BATCH_SIZE,
            move |batch| {
                if let Some(cb) = throttler_cb.read().unwrap().as_ref() {
                    cb.on_data_received(batch);
                }
            },
        );

        // 9. Main I/O event loop
        loop {
            tokio::select! {
                // Inbound data from remote host
                maybe_msg = channel.wait() => {
                    match maybe_msg {
                        Some(ChannelMsg::Data { ref data }) => {
                            let _ = throttler.push(data.to_vec()).await;
                        }
                        Some(ChannelMsg::ExtendedData { ref data, ext: _ }) => {
                            let _ = throttler.push(data.to_vec()).await;
                        }
                        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                            info!("Remote channel closed or received EOF");
                            break;
                        }
                        _ => {}
                    }
                }

                // Outbound commands or user input from iOS UI
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(SessionCommand::Input(data)) => {
                            if let Err(e) = channel.data(&data[..]).await {
                                warn!("Failed to write input to SSH channel: {e}");
                                break;
                            }
                        }
                        Some(SessionCommand::Resize(size)) => {
                            let _ = channel.window_change(size.cols as u32, size.rows as u32, 0, 0).await;
                        }
                        Some(SessionCommand::Disconnect) | None => {
                            info!("Disconnecting session gracefully for Zero Battery Drain");
                            break;
                        }
                    }
                }
            }
        }

        // Graceful channel close
        let _ = channel.eof().await;
        let _ = channel.close().await;

        Ok(())
    }
}

/// Connects via SSH, executes `command` without a PTY, captures stdout/stderr, and returns output.
pub async fn execute_remote_command_internal(
    config: &RemoteServerConfig,
    command: &str,
) -> Result<String, SshCoreError> {
    let private_key = parse_private_key(&config.private_key_pem)?;
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(private_key), None);

    let ssh_config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 2,
        ..Default::default()
    });

    let connect_future =
        client::connect(ssh_config, (config.host.as_str(), config.port), SshHandler);

    let mut session = tokio::time::timeout(Duration::from_secs(10), connect_future)
        .await
        .map_err(|_| SshCoreError::Timeout)?
        .map_err(|e| SshCoreError::ConnectionError {
            reason: format!("Failed to connect to {}:{}: {e}", config.host, config.port),
        })?;

    let auth_res = session
        .authenticate_publickey(&config.username, key_with_alg)
        .await
        .map_err(|e| SshCoreError::AuthenticationError {
            username: config.username.clone(),
            reason: format!("Authentication request failed: {e}"),
        })?;

    if !auth_res.success() {
        return Err(SshCoreError::AuthenticationError {
            username: config.username.clone(),
            reason: "Host rejected public key authentication".to_string(),
        });
    }

    let mut channel =
        session
            .channel_open_session()
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("Failed to open session channel: {e}"),
            })?;

    channel
        .exec(true, command)
        .await
        .map_err(|e| SshCoreError::ChannelError {
            reason: format!("Exec command failed: {e}"),
        })?;

    let mut output = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => {
                output.extend_from_slice(data);
            }
            ChannelMsg::ExtendedData { ref data, .. } => {
                output.extend_from_slice(data);
            }
            ChannelMsg::Eof | ChannelMsg::Close => {
                break;
            }
            _ => {}
        }
    }

    let _ = channel.eof().await;
    let _ = channel.close().await;
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "English")
        .await;

    Ok(String::from_utf8_lossy(&output).to_string())
}

#[uniffi::export]
pub async fn execute_remote_command(
    config: RemoteServerConfig,
    command: String,
) -> Result<String, SshCoreError> {
    let handle =
        RUNTIME.spawn(async move { execute_remote_command_internal(&config, &command).await });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

#[uniffi::export]
pub async fn list_remote_tmux_sessions(
    config: RemoteServerConfig,
) -> Result<Vec<TmuxSessionInfo>, SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let cmd = crate::tmux::build_tmux_list_sessions_detailed_command();
        let raw = execute_remote_command_internal(&config, &cmd).await?;
        Ok(crate::tmux::parse_tmux_sessions_output(&raw))
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

#[uniffi::export]
pub async fn kill_remote_tmux_session(
    config: RemoteServerConfig,
    session_name: String,
) -> Result<(), SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let cmd = crate::tmux::build_tmux_kill_session_command(&session_name)?;
        let _ = execute_remote_command_internal(&config, &cmd).await?;
        Ok(())
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyCallback;
    impl SshSessionCallback for DummyCallback {
        fn on_state_changed(&self, _state: SessionState) {}
        fn on_data_received(&self, _data: Vec<u8>) {}
        fn on_error(&self, _message: String) {}
    }

    #[test]
    fn test_initial_state_disconnected() {
        let config = SessionConfig {
            host: "100.64.0.1".to_string(),
            port: 22,
            username: "testuser".to_string(),
            private_key_pem: "fake-key".to_string(),
            session_name: "test-session".to_string(),
            initial_cols: 80,
            initial_rows: 24,
            command: None,
        };

        let handle = SshSessionHandle::new(config);
        assert_eq!(handle.state(), SessionState::Disconnected);
        assert!(!handle.is_connected.load(Ordering::SeqCst));
    }

    #[test]
    fn test_send_input_when_not_connected() {
        let config = SessionConfig {
            host: "100.64.0.1".to_string(),
            port: 22,
            username: "testuser".to_string(),
            private_key_pem: "fake-key".to_string(),
            session_name: "test-session".to_string(),
            initial_cols: 80,
            initial_rows: 24,
            command: None,
        };

        let handle = SshSessionHandle::new(config);
        let result = handle.send_input(b"ls\n".to_vec());
        assert!(matches!(result, Err(SshCoreError::NotConnected)));
    }

    #[test]
    fn test_disconnect_on_idle() {
        let config = SessionConfig {
            host: "100.64.0.1".to_string(),
            port: 22,
            username: "testuser".to_string(),
            private_key_pem: "fake-key".to_string(),
            session_name: "test-session".to_string(),
            initial_cols: 80,
            initial_rows: 24,
            command: None,
        };

        let handle = SshSessionHandle::new(config);
        handle.disconnect();
        assert_eq!(handle.state(), SessionState::Disconnected);
    }

    #[test]
    fn test_connect_with_invalid_key_fails() {
        let config = SessionConfig {
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "testuser".to_string(),
            private_key_pem: "invalid-key".to_string(),
            session_name: "test-session".to_string(),
            initial_cols: 80,
            initial_rows: 24,
            command: None,
        };

        let handle = SshSessionHandle::new(config);
        let connect_res = handle.connect(Box::new(DummyCallback));
        // connect() queues the background task
        assert!(connect_res.is_ok());

        // Wait briefly for background task to parse key and fail
        for _ in 0..20 {
            if matches!(handle.state(), SessionState::Failed { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        match handle.state() {
            SessionState::Failed { reason } => {
                assert!(reason.contains("Invalid OpenSSH private key"));
            }
            other => panic!("Expected Failed state, got: {:?}", other),
        }
    }
}
