use crate::buffer::{OutputThrottler, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_BATCH_SIZE};
use crate::config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
use crate::error::SshCoreError;
use crate::port_forward::{PortForwardHandle, PortForwardInfo};
use crate::tmux::TmuxSessionInfo;
use crate::transport::{
    collect_output, connect_authenticated, exec_collect, exec_start, ClientHandle, Endpoint,
    ExecOutput, DEFAULT_EXEC_TIMEOUT,
};
use russh::ChannelMsg;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub(crate) static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
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

/// A panic in one caller (or in a foreign callback) must not poison the session for everyone
/// else: the guarded data is always left in a consistent state, so recover the guard.
trait LockExt<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}
impl<T> LockExt<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn read_ok<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

fn write_ok<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

type CallbackSlot = Arc<RwLock<Option<Arc<dyn SshSessionCallback>>>>;

/// Calls the callback without holding the slot's lock, so a callback that re-enters the session
/// (e.g. to reconnect) cannot deadlock against `connect`.
fn notify(slot: &CallbackSlot, f: impl FnOnce(&dyn SshSessionCallback)) {
    let cb = read_ok(slot).clone();
    if let Some(cb) = cb {
        f(cb.as_ref());
    }
}

enum SessionCommand {
    Input(Vec<u8>),
    Resize(TerminalSize),
    Disconnect,
}

#[derive(uniffi::Object)]
pub struct SshSessionHandle {
    pub(crate) config: RwLock<SessionConfig>,
    state: Arc<RwLock<SessionState>>,
    callback: CallbackSlot,
    command_tx: Arc<Mutex<Option<mpsc::Sender<SessionCommand>>>>,
    session_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    is_connected: Arc<AtomicBool>,
    port_forwards: Arc<Mutex<Vec<Arc<PortForwardHandle>>>>,
    ssh_client: Arc<tokio::sync::Mutex<Option<ClientHandle>>>,
    host_key: Arc<Mutex<Option<String>>>,
    /// Uploads in flight by the id the app gave them: set to `true` to stop one (`cancel_upload`).
    pub(crate) upload_cancels: Arc<Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>>,
}

#[uniffi::export(async_runtime = "tokio")]
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
            host_key: Arc::new(Mutex::new(None)),
            upload_cancels: Arc::new(Mutex::new(std::collections::HashMap::new())),
        })
    }

    /// Establishes the SSH connection, launches tmux, and starts streaming I/O.
    pub fn connect(&self, callback: Box<dyn SshSessionCallback>) -> Result<(), SshCoreError> {
        if self.is_connected.load(Ordering::SeqCst) {
            return Err(SshCoreError::AlreadyConnected);
        }

        *write_ok(&self.callback) = Some(Arc::from(callback));
        self.set_state(SessionState::Connecting);

        let config = read_ok(&self.config).clone();
        self.spawn_session_loop(config)
    }

    /// Sends raw user keystrokes / input bytes to the remote terminal.
    pub fn send_input(&self, data: Vec<u8>) -> Result<(), SshCoreError> {
        if !self.is_connected.load(Ordering::SeqCst) {
            return Err(SshCoreError::NotConnected);
        }
        match self.command_tx.lock_ok().as_ref() {
            Some(tx) => {
                tx.try_send(SessionCommand::Input(data))
                    .map_err(|_| SshCoreError::ChannelError {
                        reason: "Input queue is full or closed".to_string(),
                    })
            }
            None => Err(SshCoreError::NotConnected),
        }
    }

    /// Dynamically resizes the remote terminal viewport (triggers remote SIGWINCH in tmux).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), SshCoreError> {
        let size = TerminalSize { cols, rows };
        if let Some(tx) = self.command_tx.lock_ok().as_ref() {
            let _ = tx.try_send(SessionCommand::Resize(size));
        }
        Ok(())
    }

    /// Zero Battery Drain: gracefully closes the SSH channel, port forwards, and TCP socket.
    /// tmux continues running remote agent tasks detached on the MacBook.
    pub fn disconnect(&self) {
        self.stop_all_port_forwards();

        if let Some(tx) = self.command_tx.lock_ok().take() {
            let _ = tx.try_send(SessionCommand::Disconnect);
        }

        if let Some(task) = self.session_task.lock_ok().take() {
            task.abort();
        }

        self.is_connected.store(false, Ordering::SeqCst);
        // The aborted task never reaches its own cleanup, and the client handle is what keeps the
        // TCP socket (and its keepalives) alive: drop it here.
        match self.ssh_client.try_lock() {
            Ok(mut slot) => *slot = None,
            Err(_) => {
                let slot = Arc::clone(&self.ssh_client);
                RUNTIME.spawn(async move { *slot.lock().await = None });
            }
        }
        self.set_state(SessionState::Disconnected);
    }

    /// Instant reconnection on app foreground (didBecomeActive).
    /// Re-attaches to the existing detached tmux session.
    pub fn reconnect(&self) -> Result<(), SshCoreError> {
        if self.is_connected.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.set_state(SessionState::Reconnecting);
        let config = read_ok(&self.config).clone();
        self.spawn_session_loop(config)
    }

    /// Returns current state.
    pub fn state(&self) -> SessionState {
        read_ok(&self.state).clone()
    }

    /// SHA-256 fingerprint (`SHA256:...`) of the host key the server presented on the last
    /// connection. Pin it in the server profile after the first successful connect and pass it
    /// back as `pinned_host_key`, so a changed key (possible MITM) is refused from then on.
    pub fn host_key_fingerprint(&self) -> Option<String> {
        self.host_key.lock_ok().clone()
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

    /// Executes a command on the remote host over the currently active SSH session (ephemeral
    /// channel). Output is capped at 4 MB; a command that does not finish within two minutes
    /// fails with `Timeout`.
    pub async fn execute_command_on_active_session(
        &self,
        command: String,
    ) -> Result<String, SshCoreError> {
        self.exec_with_timeout(&command, DEFAULT_EXEC_TIMEOUT).await
    }
}

impl SshSessionHandle {
    fn set_state(&self, new_state: SessionState) {
        *write_ok(&self.state) = new_state.clone();
        notify(&self.callback, |cb| cb.on_state_changed(new_state));
    }

    /// Opens the channel under the connection lock (short), then reads it without the lock so
    /// several commands can run at once and `disconnect` is never blocked behind one.
    pub(crate) async fn exec_raw(
        &self,
        command: &str,
        limit: Duration,
    ) -> Result<ExecOutput, SshCoreError> {
        let channel = {
            let slot = self.ssh_client.lock().await;
            let handle = slot.as_ref().ok_or(SshCoreError::NotConnected)?;
            exec_start(handle, command).await?
        };
        collect_output(channel, limit).await
    }

    /// Like `exec_raw`, but sends `input` on the channel's stdin and then EOF. For values that must
    /// not appear in a command line (the relay secret): the command reads them from stdin.
    pub(crate) async fn exec_raw_with_stdin(
        &self,
        command: &str,
        input: &[u8],
        limit: Duration,
    ) -> Result<ExecOutput, SshCoreError> {
        let channel = {
            let slot = self.ssh_client.lock().await;
            let handle = slot.as_ref().ok_or(SshCoreError::NotConnected)?;
            exec_start(handle, command).await?
        };
        channel
            .data(input)
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("Could not send input: {e}"),
            })?;
        let _ = channel.eof().await;
        collect_output(channel, limit).await
    }

    pub(crate) async fn exec_with_stdin_text(
        &self,
        command: &str,
        input: &[u8],
    ) -> Result<String, SshCoreError> {
        Ok(self
            .exec_raw_with_stdin(command, input, DEFAULT_EXEC_TIMEOUT)
            .await?
            .text())
    }

    /// Opens an exec channel the caller reads itself (long-lived streams).
    pub(crate) async fn exec_open(
        &self,
        command: &str,
    ) -> Result<russh::Channel<russh::client::Msg>, SshCoreError> {
        let slot = self.ssh_client.lock().await;
        let handle = slot.as_ref().ok_or(SshCoreError::NotConnected)?;
        exec_start(handle, command).await
    }

    pub(crate) async fn exec_with_timeout(
        &self,
        command: &str,
        limit: Duration,
    ) -> Result<String, SshCoreError> {
        Ok(self.exec_raw(command, limit).await?.text())
    }

    fn spawn_session_loop(&self, config: SessionConfig) -> Result<(), SshCoreError> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(256);
        *self.command_tx.lock_ok() = Some(cmd_tx);

        let callback = Arc::clone(&self.callback);
        let is_connected_flag = Arc::clone(&self.is_connected);
        let state_arc = Arc::clone(&self.state);
        let ssh_client_arc = Arc::clone(&self.ssh_client);
        let host_key = Arc::clone(&self.host_key);

        // Set before the task runs so `send_input` right after `connect` is not refused.
        self.is_connected.store(true, Ordering::SeqCst);
        let session_task = RUNTIME.spawn(async move {
            let run_result = run_ssh_connection(
                &config,
                &mut cmd_rx,
                &callback,
                &state_arc,
                &ssh_client_arc,
                &host_key,
            )
            .await;

            *ssh_client_arc.lock().await = None;
            is_connected_flag.store(false, Ordering::SeqCst);

            let final_state = match run_result {
                Ok(()) => SessionState::Disconnected,
                Err(e) => {
                    let err_msg = e.to_string();
                    notify(&callback, |cb| cb.on_error(err_msg.clone()));
                    SessionState::Failed { reason: err_msg }
                }
            };

            *write_ok(&state_arc) = final_state.clone();
            notify(&callback, |cb| cb.on_state_changed(final_state));
        });

        *self.session_task.lock_ok() = Some(session_task);
        Ok(())
    }
}

async fn run_ssh_connection(
    config: &SessionConfig,
    cmd_rx: &mut mpsc::Receiver<SessionCommand>,
    callback: &CallbackSlot,
    state_arc: &Arc<RwLock<SessionState>>,
    ssh_client_slot: &Arc<tokio::sync::Mutex<Option<ClientHandle>>>,
    host_key: &Arc<Mutex<Option<String>>>,
) -> Result<(), SshCoreError> {
    debug!("Connecting to {}:{}", config.host, config.port);
    let conn = connect_authenticated(
        &Endpoint {
            host: &config.host,
            port: config.port,
            username: &config.username,
            private_key_pem: &config.private_key_pem,
            pinned_host_key: config.pinned_host_key.as_deref(),
        },
        Duration::from_secs(30),
        3,
    )
    .await?;
    *host_key.lock_ok() = Some(conn.host_key.clone());

    // Open the interactive terminal channel.
    let mut channel =
        conn.handle
            .channel_open_session()
            .await
            .map_err(|e| SshCoreError::ChannelError {
                reason: format!("Failed to open session channel: {e}"),
            })?;

    // The handle serves port forwarding and ephemeral exec channels from here on.
    *ssh_client_slot.lock().await = Some(conn.handle);

    // PTY with xterm-256color for TrueColor support.
    channel
        .request_pty(
            false,
            "xterm-256color",
            u32::from(config.initial_cols),
            u32::from(config.initial_rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(|e| SshCoreError::ChannelError {
            reason: format!("PTY request failed: {e}"),
        })?;

    // Dynamic login shell + tmux attach/create command.
    let tmux_cmd = config.multiplexer.attach_or_create(
        &config.session_name,
        config.command.as_deref(),
        config.start_dir.as_deref(),
    )?;
    channel
        .exec(true, tmux_cmd)
        .await
        .map_err(|e| SshCoreError::ChannelError {
            reason: format!("Exec command failed: {e}"),
        })?;

    *write_ok(state_arc) = SessionState::Connected;
    notify(callback, |cb| cb.on_state_changed(SessionState::Connected));

    // The throttler batches output so SwiftTerm is not flooded.
    let throttler_cb = Arc::clone(callback);
    let throttler = OutputThrottler::new(
        DEFAULT_FLUSH_INTERVAL,
        DEFAULT_MAX_BATCH_SIZE,
        move |batch| notify(&throttler_cb, |cb| cb.on_data_received(batch)),
    );

    loop {
        tokio::select! {
            maybe_msg = channel.wait() => match maybe_msg {
                Some(ChannelMsg::Data { ref data } | ChannelMsg::ExtendedData { ref data, .. }) => {
                    let _ = throttler.push(data.to_vec()).await;
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                    info!("Remote channel closed or received EOF");
                    break;
                }
                _ => {}
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(SessionCommand::Input(data)) => {
                    if let Err(e) = channel.data(&data[..]).await {
                        warn!("Failed to write input to SSH channel: {e}");
                        break;
                    }
                }
                Some(SessionCommand::Resize(size)) => {
                    let _ = channel
                        .window_change(u32::from(size.cols), u32::from(size.rows), 0, 0)
                        .await;
                }
                Some(SessionCommand::Disconnect) | None => {
                    info!("Disconnecting session gracefully for Zero Battery Drain");
                    break;
                }
            },
        }
    }

    let _ = channel.eof().await;
    let _ = channel.close().await;
    Ok(())
}

/// Connects via SSH, executes `command` without a PTY, captures stdout/stderr, and returns output.
pub async fn execute_remote_command_internal(
    config: &RemoteServerConfig,
    command: &str,
) -> Result<String, SshCoreError> {
    let conn = connect_authenticated(
        &Endpoint {
            host: &config.host,
            port: config.port,
            username: &config.username,
            private_key_pem: &config.private_key_pem,
            pinned_host_key: config.pinned_host_key.as_deref(),
        },
        Duration::from_secs(10),
        2,
    )
    .await?;
    let out = exec_collect(&conn.handle, command, DEFAULT_EXEC_TIMEOUT).await;
    let _ = conn
        .handle
        .disconnect(russh::Disconnect::ByApplication, "", "English")
        .await;
    Ok(out?.text())
}

#[uniffi::export(async_runtime = "tokio")]
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

#[uniffi::export(async_runtime = "tokio")]
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

/// Which system a host runs, from one short connection (before a terminal session exists). It is
/// advice for choosing the multiplexer and the helper build, never trusted for security.
#[uniffi::export(async_runtime = "tokio")]
pub async fn detect_host_target_once(
    config: RemoteServerConfig,
) -> Result<crate::hosttarget::HostTarget, SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let out = execute_remote_command_internal(
            &config,
            &crate::hostprobe::login_sh_script(crate::hosttarget::HOST_TARGET_SCRIPT, &[], 10),
        )
        .await
        .unwrap_or_default();
        let target = crate::hosttarget::parse_host_target(out);
        if let crate::hosttarget::HostTarget::Unsupported { reason } = &target {
            if reason.starts_with("Windows without WSL2") {
                let out = execute_remote_command_internal(
                    &config,
                    crate::hosttarget::WINDOWS_TARGET_SCRIPT,
                )
                .await
                .unwrap_or_default();
                if let Some(windows) = crate::hosttarget::parse_windows_target(&out) {
                    return Ok(windows);
                }
            }
        }
        Ok(target)
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

/// Puts the Windows helper (which IS the multiplexer on a native Windows host) in place if the
/// verified build is not there yet. One short connection; the install is a few hundred small
/// commands, so it runs once per app version and host.
#[uniffi::export(async_runtime = "tokio")]
pub async fn ensure_windows_helper_once(config: RemoteServerConfig) -> Result<(), SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let target = crate::hosttarget::HostTarget::WindowsX86_64;
        let sha = crate::usage::bundled_host_helper_sha256_for(target.clone()).ok_or(
            SshCoreError::InvalidInput {
                reason: "This build of the app has no Windows helper".to_string(),
            },
        )?;
        let check = crate::windows_install::build_check_command(&sha)?;
        let present = execute_remote_command_internal(&config, &check)
            .await
            .ok()
            .and_then(|o| crate::windows_install::parse_certutil_hash(&o));
        if present.as_deref() == Some(sha.as_str()) {
            return Ok(());
        }
        let bytes = crate::usage::bundled_helper_for(&target).unwrap_or_default();
        let mut last = String::new();
        for cmd in crate::windows_install::build_install_commands(bytes, &sha)? {
            last = execute_remote_command_internal(&config, &cmd).await?;
        }
        if crate::windows_install::parse_certutil_hash(&last).as_deref() == Some(sha.as_str()) {
            Ok(())
        } else {
            // A wrong file must not stay under the name the app starts.
            let _ = execute_remote_command_internal(
                &config,
                &format!("del {}", crate::windows_install::helper_path(&sha)),
            )
            .await;
            Err(SshCoreError::IoError {
                reason: "The Windows helper did not install (is cmd.exe the SSH shell? See the Windows setup)"
                    .to_string(),
            })
        }
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

/// Sessions on a host, for the kind of multiplexer it runs (`HostTarget::multiplexer`).
#[uniffi::export(async_runtime = "tokio")]
pub async fn list_remote_sessions(
    config: RemoteServerConfig,
    multiplexer: crate::multiplexer::MultiplexerKind,
) -> Result<Vec<TmuxSessionInfo>, SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let raw = execute_remote_command_internal(&config, &multiplexer.list_detailed()).await?;
        Ok(crate::tmux::parse_tmux_sessions_output(&raw))
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

/// Ends one session, for the kind of multiplexer the host runs.
#[uniffi::export(async_runtime = "tokio")]
pub async fn kill_remote_session(
    config: RemoteServerConfig,
    session_name: String,
    multiplexer: crate::multiplexer::MultiplexerKind,
) -> Result<(), SshCoreError> {
    let handle = RUNTIME.spawn(async move {
        let cmd = multiplexer.kill(&session_name)?;
        let _ = execute_remote_command_internal(&config, &cmd).await?;
        Ok(())
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

/// One snapshot of the host without a live terminal session: a single short-lived connection.
/// This is the entry point for anything that cannot keep a session open (a home-screen widget).
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_host_telemetry_once(
    config: RemoteServerConfig,
    helper_sha256: Option<String>,
) -> Result<crate::telemetry::HostTelemetrySnapshot, SshCoreError> {
    if let Some(pin) = &helper_sha256 {
        crate::usage::validate_sha256(pin)?;
    }
    let handle = RUNTIME.spawn(async move {
        if let Some(pin) = &helper_sha256 {
            let out = execute_remote_command_internal(
                &config,
                &crate::telemetry::build_helper_telemetry_command(pin),
            )
            .await?;
            if let Some(snap) = crate::telemetry::parse_helper_telemetry(&out) {
                return Ok(snap);
            }
        }
        let raw =
            execute_remote_command_internal(&config, crate::telemetry::TELEMETRY_COLLECTION_SCRIPT)
                .await?;
        Ok(crate::telemetry::parse_host_telemetry(raw))
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

/// Which models run in the host's tmux sessions and how much context / usage they have used.
/// `config_dirs` are the Claude config directories of the saved profiles (empty = `~/.claude`).
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_host_usage(
    config: RemoteServerConfig,
    config_dirs: Vec<String>,
    helper_sha256: Option<String>,
) -> Result<crate::usage::HostUsageReport, SshCoreError> {
    crate::usage::validate_config_dirs(&config_dirs)?;
    if let Some(pin) = &helper_sha256 {
        crate::usage::validate_sha256(pin)?;
    }
    let handle = RUNTIME.spawn(async move {
        let cmd = crate::usage::build_usage_command(&config_dirs, helper_sha256.as_deref());
        let raw = execute_remote_command_internal(&config, &cmd).await?;
        match crate::usage::parse_usage_report(&raw) {
            Ok(report) => Ok(report),
            // A helper that answered with something unreadable must not break the usage line.
            Err(_) if helper_sha256.is_some() => {
                let cmd = crate::usage::build_usage_command(&config_dirs, None);
                let raw = execute_remote_command_internal(&config, &cmd).await?;
                crate::usage::parse_usage_report(&raw)
            }
            Err(e) => Err(e),
        }
    });
    handle.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })?
}

#[uniffi::export(async_runtime = "tokio")]
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
            start_dir: None,
            pinned_host_key: None,
            multiplexer: Default::default(),
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
            start_dir: None,
            pinned_host_key: None,
            multiplexer: Default::default(),
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
            start_dir: None,
            pinned_host_key: None,
            multiplexer: Default::default(),
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
            start_dir: None,
            pinned_host_key: None,
            multiplexer: Default::default(),
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
