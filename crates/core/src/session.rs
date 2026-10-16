use crate::buffer::{OutputThrottler, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_BATCH_SIZE};
use crate::config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
use crate::error::SshCoreError;
use crate::port_forward::{PortForwardHandle, PortForwardInfo};
use crate::tmux::{build_tmux_attach_or_create_command, TmuxSessionInfo};
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

fn read_ok<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
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
    config: RwLock<SessionConfig>,
    state: Arc<RwLock<SessionState>>,
    callback: CallbackSlot,
    command_tx: Arc<Mutex<Option<mpsc::Sender<SessionCommand>>>>,
    session_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    is_connected: Arc<AtomicBool>,
    port_forwards: Arc<Mutex<Vec<Arc<PortForwardHandle>>>>,
    ssh_client: Arc<tokio::sync::Mutex<Option<ClientHandle>>>,
    host_key: Arc<Mutex<Option<String>>>,
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
            host_key: Arc::new(Mutex::new(None)),
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
            "curl -s --max-time 290 -X POST http://127.0.0.1:11434/api/generate -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload_str)
        );

        // Local models can be slow, but not forever.
        let raw_response = self
            .exec_with_timeout(&curl_cmd, Duration::from_secs(300))
            .await?;
        let generated_text = crate::ai::parse_ollama_generate_response(&raw_response)?;
        crate::ai::parse_ai_response(&generated_text)
    }

    /// Reads and parses the latest transcript from the remote host.
    pub async fn read_remote_transcript(
        &self,
        remote_path: String,
    ) -> Result<Vec<crate::transcript::TranscriptTurn>, SshCoreError> {
        crate::transcript::validate_transcript_path(&remote_path).map_err(|e| {
            SshCoreError::TranscriptError {
                reason: e.to_string(),
            }
        })?;

        let tail_cmd = format!("tail -n 500 -- {}", crate::tmux::shell_quote(&remote_path));
        let raw_output = self.execute_command_on_active_session(tail_cmd).await?;

        crate::transcript::parse_transcript_jsonl(raw_output).map_err(|e| {
            SshCoreError::TranscriptError {
                reason: e.to_string(),
            }
        })
    }

    /// Automatically detects the path of the active agent transcript on the remote host.
    pub async fn detect_active_transcript_path(
        &self,
        base_dir: Option<String>,
    ) -> Result<Option<String>, SshCoreError> {
        if let Some(ref dir) = base_dir {
            crate::transcript::validate_transcript_path(dir).map_err(|e| {
                SshCoreError::TranscriptError {
                    reason: e.to_string(),
                }
            })?;
        }
        let quoted_dir = match base_dir {
            Some(dir) => crate::tmux::shell_quote(&dir),
            None => "\"$HOME\"".to_string(),
        };
        let detect_script =
            crate::hostprobe::login_sh_script(TRANSCRIPT_DETECT_SCRIPT, &[quoted_dir], 10);

        let output = self
            .execute_command_on_active_session(detect_script)
            .await?;
        let trimmed = output.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    /// Reads recent commands from the remote host's shell history (`~/.zsh_history` / `~/.bash_history`)
    /// and scrubs sensitive credentials before returning.
    pub async fn fetch_remote_shell_history(
        &self,
        limit: u32,
    ) -> Result<Vec<String>, SshCoreError> {
        let safe_limit = limit.clamp(10, 1000);
        let cmd = format!(
            "tail -n {} \"$HOME/.zsh_history\" \"$HOME/.bash_history\" 2>/dev/null || true",
            safe_limit * 3
        );
        let raw_output = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::snippets::extract_history_commands(
            raw_output, safe_limit,
        ))
    }

    /// Collects a snapshot of remote host telemetry (CPU load, RAM, Disk, Battery, and AI/dev processes).
    pub async fn fetch_host_telemetry(
        &self,
    ) -> Result<crate::telemetry::HostTelemetrySnapshot, SshCoreError> {
        let raw_output = self
            .execute_command_on_active_session(
                crate::telemetry::TELEMETRY_COLLECTION_SCRIPT.to_string(),
            )
            .await?;
        Ok(crate::telemetry::parse_host_telemetry(raw_output))
    }

    /// Safely terminates a remote process by PID (`SIGTERM` or `SIGKILL`), rejecting reserved PIDs <= 1.
    pub async fn kill_remote_process(&self, pid: u32, force: bool) -> Result<(), SshCoreError> {
        let safe_pid = crate::telemetry::validate_kill_pid(pid)?;
        let sig = if force { "-KILL" } else { "-TERM" };
        let out = self
            .exec_raw(&format!("kill {sig} -- {safe_pid}"), DEFAULT_EXEC_TIMEOUT)
            .await?;
        match out.exit_status {
            Some(0) => Ok(()),
            _ => Err(SshCoreError::IoError {
                reason: format!(
                    "kill failed on the host: {}",
                    crate::approval_queue::display_safe(
                        &out.text().trim().chars().take(200).collect::<String>()
                    )
                ),
            }),
        }
    }

    /// Lists entries in a remote directory (or active tmux pane working directory if `path` is `None`).
    pub async fn list_remote_directory(
        &self,
        path: Option<String>,
        include_hidden: bool,
    ) -> Result<crate::file_browser::RemoteDirectoryListing, SshCoreError> {
        let session_name = read_ok(&self.config).session_name.clone();
        let cmd =
            crate::file_browser::build_list_directory_command(path.as_deref(), &session_name)?;
        let raw_output = self.execute_command_on_active_session(cmd).await?;
        let listing =
            crate::file_browser::parse_remote_directory_listing(raw_output, include_hidden);
        // The shell resolved symlinks (`pwd -P`); re-check the real location against the blocklist.
        crate::file_browser::validate_remote_dir_path(&listing.current_path)?;
        Ok(listing)
    }

    /// Resolves `binary` in a login shell and in an interactive shell (rc files loaded), so the
    /// app can tell the user whether the agent is only reachable after sourcing `~/.zshrc`.
    pub async fn check_agent_environment(
        &self,
        binary: String,
    ) -> Result<crate::agents::AgentEnvCheck, SshCoreError> {
        // A user's rc file can block (prompts, slow plugins); never let the UI wait on it forever.
        const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
        let login = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.execute_command_on_active_session(crate::agents::build_binary_probe_command(
                binary.clone(),
                false,
            )?),
        )
        .await
        .map_err(|_| SshCoreError::Timeout)??;
        let interactive = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.execute_command_on_active_session(crate::agents::build_binary_probe_command(
                binary, true,
            )?),
        )
        .await;
        Ok(crate::agents::AgentEnvCheck {
            login_path: crate::agents::parse_resolved_binary(login),
            interactive_timed_out: interactive.is_err(),
            interactive_path: interactive
                .ok()
                .transpose()?
                .and_then(crate::agents::parse_resolved_binary),
        })
    }

    /// Checks the host for Ollama: installed, server reachable on 127.0.0.1:11434, model list.
    /// A timeout is an error (the host may be slow), not \"not installed\".
    pub async fn check_host_ollama(&self) -> Result<crate::ai::OllamaHostStatus, SshCoreError> {
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            self.execute_command_on_active_session(crate::ai::build_ollama_probe_command()),
        )
        .await
        .map_err(|_| SshCoreError::Timeout)??;
        Ok(crate::ai::parse_ollama_probe(out))
    }

    /// Reports whether the ssh-app hooks are present in the agent's config and the script is installed.
    pub async fn agent_hooks_status(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        hook_dir: Option<String>,
    ) -> Result<crate::agents::AgentHooksStatus, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        let hook_dir = hook_dir.unwrap_or_else(|| crate::agents::DEFAULT_HOOK_DIR.to_string());
        let script = crate::agents::remote_script_path(&hook_dir);
        let present = self
            .execute_command_on_active_session(format!("[ -f {script} ] && echo yes || echo no"))
            .await?;
        Ok(crate::agents::AgentHooksStatus {
            hooks_installed: crate::agents::agent_hooks_installed(agent, existing),
            config_path,
            script_installed: present.trim() == "yes",
        })
    }

    /// Installs the default (phone approval) hooks; see `install_agent_hooks_with_preset`.
    pub async fn install_agent_hooks(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        hook_dir: Option<String>,
    ) -> Result<(), SshCoreError> {
        self.install_agent_hooks_with_preset(
            agent,
            config_dir,
            hook_dir,
            crate::agents::default_hook_preset(),
        )
        .await
    }

    /// Dry run: the hooks section a preset would produce (never other settings), without touching
    /// the host. Lets the user review exactly what will be installed.
    pub async fn preview_agent_hooks(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        hook_dir: Option<String>,
        preset: crate::agents::HookPreset,
    ) -> Result<String, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let planned = crate::agents::plan_agent_hooks(agent, preset, hook_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        let merged = crate::agents::merge_planned_hooks(agent, existing, planned)?;
        // Only the hooks part: settings.json may hold API keys that must not reach the screen.
        crate::agents::hooks_section_of(agent, merged)
    }

    /// The permission rules (allow / ask / deny / default mode) in the profile's `settings.json`.
    /// Refuses a file it cannot represent exactly (see `permissions_from_settings`).
    pub async fn read_agent_permissions(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
    ) -> Result<crate::permissions::AgentPermissions, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        crate::permissions::permissions_from_settings(agent, existing)
    }

    /// Dry run: only the resulting `permissions` object, never other settings (they may hold keys).
    pub async fn preview_agent_permissions(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        permissions: crate::permissions::AgentPermissions,
    ) -> Result<String, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        let merged = crate::permissions::merge_permissions(agent, existing, permissions)?;
        crate::permissions::permissions_section_of(agent, merged)
    }

    /// Replaces the four permission fields in the profile's `settings.json` (backup once, atomic,
    /// symlink- and mode-preserving). Refuses an unparsable file and leaves it untouched.
    pub async fn apply_agent_permissions(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        permissions: crate::permissions::AgentPermissions,
    ) -> Result<(), SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        let merged = crate::permissions::merge_permissions(agent, existing.clone(), permissions)?;
        self.write_remote_config(&config_path, &existing, &merged)
            .await
    }

    /// The ssh-app hook entries currently installed in the agent's config.
    pub async fn installed_agent_hooks(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
    ) -> Result<Vec<crate::agents::PlannedHook>, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        Ok(crate::agents::installed_hook_entries(agent, existing))
    }

    /// Installs hooks for `agent`: merges the preset's entries into its config (backup, atomic,
    /// symlink- and mode-preserving) and, unless the preset is `Custom`, writes the hook script.
    /// Refuses to touch an unparsable config.
    pub async fn install_agent_hooks_with_preset(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
        hook_dir: Option<String>,
        preset: crate::agents::HookPreset,
    ) -> Result<(), SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let hook_dir = hook_dir.unwrap_or_else(|| crate::agents::DEFAULT_HOOK_DIR.to_string());

        let uses_script = crate::agents::hook_preset_uses_script(preset.clone());
        let planned = crate::agents::plan_agent_hooks(agent, preset, Some(hook_dir.clone()))?;

        let py = self
            .execute_command_on_active_session(
                "python3 --version >/dev/null 2>&1 && echo ok || echo missing".to_string(),
            )
            .await?;
        if uses_script && py.trim() != "ok" {
            return Err(SshCoreError::IoError {
                reason: "python3 is required on the host to run the approval hook".to_string(),
            });
        }

        let existing = self.read_remote_config(&config_path).await?;
        let merged = crate::agents::merge_planned_hooks(agent, existing.clone(), planned)?;

        // Script first, so a config never points at a missing script.
        let script_path = crate::agents::remote_script_path(&hook_dir);
        let dir_expr = crate::agents::remote_path_expr(hook_dir.trim_end_matches('/'));
        // Only directories we create are locked down: never chmod a pre-existing (or parent) dir.
        let lock_default_parent = if hook_dir.trim_end_matches('/')
            == crate::agents::DEFAULT_HOOK_DIR
        {
            "mkdir -p \"$HOME/.ssh-app/run\" && chmod 700 \"$HOME/.ssh-app\" \"$HOME/.ssh-app/run\"; "
        } else {
            ""
        };
        if uses_script {
            self.install_approval_key().await?;
            self.execute_command_on_active_session(format!(
            "umask 077; if [ ! -d {dir_expr} ]; then mkdir -p {dir_expr} && chmod 700 {dir_expr}; fi; {lock_default_parent}{}",
            crate::agents::build_write_file_command(
                &script_path,
                &crate::agents::agent_hook_script(),
                "700"
            )
        ))
            .await?;
        }
        self.write_remote_config(&config_path, &existing, &merged)
            .await
    }

    /// Git diff (`HEAD`) of the repository the tmux session's pane is working in. The command runs
    /// in the pane's directory: an exec channel starts in `$HOME`, where `git diff` just prints
    /// its usage text.
    pub async fn fetch_git_diff(
        &self,
        session_name: String,
    ) -> Result<crate::gitdiff::GitDiffResult, SshCoreError> {
        let cmd = crate::gitdiff::build_git_diff_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitdiff::parse_git_diff_output(&out))
    }

    /// Pending approval requests and notices on the host, with the host's clock. An empty list
    /// when the queue is not installed (see `approval_status`).
    pub async fn list_pending_approvals(
        &self,
    ) -> Result<crate::approval_queue::ApprovalList, SshCoreError> {
        use crate::approval_queue::{build_ctl_command, parse_approval_list, CtlSubcommand};
        let out = self
            .execute_command_on_active_session(build_ctl_command(CtlSubcommand::List))
            .await?;
        Ok(parse_approval_list(&out))
    }

    /// Signs and submits the user's decision for request `id`. The signature covers the host's
    /// own id/hash/nonce and the host clock, so the host only honours it for that exact request.
    pub async fn submit_approval_decision(
        &self,
        id: String,
        approved: bool,
    ) -> Result<(), SshCoreError> {
        use crate::approval_queue::{build_decide_command, parse_ctl_result, sign_approval_sshsig};
        let list = self.list_pending_approvals().await?;
        let item = list
            .items
            .iter()
            .find(|i| i.id == id && i.kind == "approval")
            .filter(|i| i.expires_sec > list.host_time_sec)
            .ok_or_else(|| SshCoreError::ApprovalVerificationError {
                reason: "the request is gone or has expired".to_string(),
            })?;
        let ts = list.host_time_sec.max(item.timestamp_sec);
        let message = crate::approval::create_canonical_signing_bytes(
            item.id.clone(),
            item.command_sha256.clone(),
            item.nonce.clone(),
            ts,
            approved,
        );
        let key = zeroize::Zeroizing::new(read_ok(&self.config).private_key_pem.clone());
        let signature = sign_approval_sshsig((*key).clone(), message)?;
        let out = self
            .execute_command_on_active_session(build_decide_command(
                &item.id, approved, ts, &signature,
            )?)
            .await?;
        parse_ctl_result(&out)
    }

    /// Dismisses an informational notice on the host.
    pub async fn acknowledge_notification(&self, id: String) -> Result<(), SshCoreError> {
        let cmd = crate::approval_queue::build_ack_command(&id)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        crate::approval_queue::parse_ctl_result(&out)
    }

    /// Creates the host queue directories and trusts this device's key for approvals.
    pub async fn install_approval_key(&self) -> Result<(), SshCoreError> {
        let key = zeroize::Zeroizing::new(read_ok(&self.config).private_key_pem.clone());
        let public = crate::keys::derive_public_key((*key).clone())?;
        let line = crate::approval_queue::allowed_signers_line(&public)?;
        let out = self
            .execute_command_on_active_session(crate::approval_queue::build_install_key_command(
                &line,
            ))
            .await?;
        if out.trim() == "ok" {
            Ok(())
        } else {
            Err(SshCoreError::IoError {
                reason: "could not set up the approval queue on the host".to_string(),
            })
        }
    }

    /// Whether the host can verify phone approvals (OpenSSH `ssh-keygen`, trusted key installed).
    pub async fn approval_status(
        &self,
    ) -> Result<crate::approval_queue::ApprovalStatus, SshCoreError> {
        use crate::approval_queue::{build_ctl_command, parse_ctl_status, CtlSubcommand};
        let out = self
            .execute_command_on_active_session(build_ctl_command(CtlSubcommand::Status))
            .await?;
        Ok(parse_ctl_status(&out))
    }

    /// Removes only the ssh-app hook entries from the agent's config (the script is left in place).
    pub async fn uninstall_agent_hooks(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
    ) -> Result<(), SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        if existing.trim().is_empty() {
            return Ok(());
        }
        let cleaned = crate::agents::remove_agent_hooks(agent, existing.clone())?;
        self.write_remote_config(&config_path, &existing, &cleaned)
            .await
    }

    /// Reads a preview of a remote file (up to `max_bytes`, capped at 128 KB), blocking sensitive credential files.
    pub async fn read_remote_file_preview(
        &self,
        path: String,
        max_bytes: u32,
    ) -> Result<crate::file_browser::RemoteFilePreview, SshCoreError> {
        let requested = crate::file_browser::validate_remote_file_read_path(&path)?;
        let capped_max = (max_bytes as usize).clamp(1024, 131_072);
        // Resolve symlinks first so a harmless-looking link cannot expose a blocked file.
        let resolved = self
            .execute_command_on_active_session(crate::file_browser::build_resolve_path_command(
                &requested,
            ))
            .await?;
        let resolved = resolved.trim();
        if resolved.is_empty() {
            return Err(SshCoreError::IoError {
                reason: "File not found".to_string(),
            });
        }
        let safe_path = crate::file_browser::validate_remote_file_read_path(resolved)?;
        let cmd = crate::file_browser::build_read_file_command(&safe_path, capped_max);
        let raw_output = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::file_browser::build_file_preview_from_bytes(
            &safe_path,
            raw_output.as_bytes(),
            capped_max,
        ))
    }
}

impl SshSessionHandle {
    async fn read_remote_config(&self, path: &str) -> Result<String, SshCoreError> {
        self.execute_command_on_active_session(format!(
            "cat {} 2>/dev/null || true",
            crate::agents::remote_path_expr(path)
        ))
        .await
    }

    /// Writes through symlinks, keeps the original mode (new files `0600`), backs up once.
    /// `previous` is what the caller read and merged into `content`: if the file on the host
    /// changed since (the agent or the user edited it), nothing is written.
    async fn write_remote_config(
        &self,
        path: &str,
        previous: &str,
        content: &str,
    ) -> Result<(), SshCoreError> {
        let cmd = build_write_config_command(path, previous, content);
        let out = self.execute_command_on_active_session(cmd).await?;
        let last = out.lines().last().unwrap_or("").trim();
        match last {
            "CONFIG_WRITTEN" => Ok(()),
            "CONFIG_CHANGED" => Err(SshCoreError::IoError {
                reason: "The config file changed on the host while you were editing it; nothing was written. Reopen and try again.".to_string(),
            }),
            _ => Err(SshCoreError::IoError {
                reason: format!(
                    "Writing the config file failed on the host: {}",
                    crate::approval_queue::display_safe(&out.chars().take(200).collect::<String>())
                ),
            }),
        }
    }

    fn set_state(&self, new_state: SessionState) {
        *write_ok(&self.state) = new_state.clone();
        notify(&self.callback, |cb| cb.on_state_changed(new_state));
    }

    /// Opens the channel under the connection lock (short), then reads it without the lock so
    /// several commands can run at once and `disconnect` is never blocked behind one.
    async fn exec_raw(&self, command: &str, limit: Duration) -> Result<ExecOutput, SshCoreError> {
        let channel = {
            let slot = self.ssh_client.lock().await;
            let handle = slot.as_ref().ok_or(SshCoreError::NotConnected)?;
            exec_start(handle, command).await?
        };
        collect_output(channel, limit).await
    }

    async fn exec_with_timeout(
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
    let tmux_cmd =
        build_tmux_attach_or_create_command(&config.session_name, config.command.as_deref())?;
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

const TRANSCRIPT_DETECT_SCRIPT: &str = r#"TARGET_DIR="$1"
if [ -d "$HOME/.gemini/antigravity-cli/brain" ]; then
  LATEST=$(ls -td "$HOME/.gemini/antigravity-cli/brain"/* 2>/dev/null | head -n 1)
  if [ -n "$LATEST" ] && [ -f "$LATEST/.system_generated/logs/transcript.jsonl" ]; then
    echo "$LATEST/.system_generated/logs/transcript.jsonl"
    exit 0
  fi
fi
if [ -d "$HOME/.claude/projects" ]; then
  LATEST_CLAUDE=$(ls -td "$HOME/.claude/projects"/*/*.jsonl 2>/dev/null | head -n 1)
  if [ -n "$LATEST_CLAUDE" ] && [ -f "$LATEST_CLAUDE" ]; then
    echo "$LATEST_CLAUDE"
    exit 0
  fi
fi
if [ -n "$TARGET_DIR" ] && [ -f "$TARGET_DIR/.agents/transcript.jsonl" ]; then
  echo "$TARGET_DIR/.agents/transcript.jsonl"
  exit 0
fi
if [ -n "$TARGET_DIR" ] && [ -f "$TARGET_DIR/transcript.jsonl" ]; then
  echo "$TARGET_DIR/transcript.jsonl"
  exit 0
fi"#;

/// Host command that replaces the config file only if it still holds `previous`; prints
/// `CONFIG_WRITTEN` or `CONFIG_CHANGED` as its last line.
pub(crate) fn build_write_config_command(path: &str, previous: &str, content: &str) -> String {
    use sha2::{Digest, Sha256};
    let expected = hex::encode(Sha256::digest(previous.as_bytes()));
    let p = crate::agents::remote_path_expr(path);
    let body = crate::tmux::shell_quote(content);
    format!(
            "set -e; p={p}; mkdir -p \"$(dirname \"$p\")\"; \
             if [ -L \"$p\" ]; then p=$(realpath -- \"$p\" 2>/dev/null || readlink -f -- \"$p\"); fi; \
             if [ -f \"$p\" ] && [ ! -e \"$p.ssh-app.bak\" ]; then cp -p \"$p\" \"$p.ssh-app.bak\"; fi; \
             t=\"$p.ssh-app.tmp\"; rm -f \"$t\"; umask 077; set -C; printf %s {body} > \"$t\"; set +C; \
             if [ -f \"$p\" ]; then chmod --reference=\"$p\" \"$t\" 2>/dev/null || chmod \"$(stat -f %Lp \"$p\" 2>/dev/null || echo 600)\" \"$t\"; fi; \
             h=$( {{ cat \"$p\" 2>/dev/null || true; }} | {{ shasum -a 256 2>/dev/null || sha256sum; }} | cut -d' ' -f1 ); \
             if [ \"$h\" != '{expected}' ]; then rm -f \"$t\"; echo CONFIG_CHANGED; exit 0; fi; \
             mv \"$t\" \"$p\" && echo CONFIG_WRITTEN"
        )
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
            pinned_host_key: None,
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
            pinned_host_key: None,
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
            pinned_host_key: None,
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
            pinned_host_key: None,
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

    #[test]
    fn config_write_refuses_when_the_file_changed_since_it_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let p = path.to_str().unwrap();
        let run = |prev: &str, new: &str| {
            let out = std::process::Command::new("sh")
                .args(["-c", &build_write_config_command(p, prev, new)])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        // New file: the expected previous content is empty.
        assert_eq!(run("", "{\"a\":1}"), "CONFIG_WRITTEN");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        // Someone edits the file after we read it: nothing is written, no temp file is left.
        std::fs::write(&path, "{\"a\":2}").unwrap();
        assert_eq!(run("{\"a\":1}", "{\"b\":1}"), "CONFIG_CHANGED");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
        assert!(!dir.path().join("settings.json.ssh-app.tmp").exists());
        // Unchanged since the read: written.
        assert_eq!(run("{\"a\":2}", "{\"b\":1}"), "CONFIG_WRITTEN");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"b\":1}");
    }
}
