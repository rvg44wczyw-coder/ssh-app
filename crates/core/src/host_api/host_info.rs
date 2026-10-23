use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Collects a snapshot of remote host telemetry (CPU load, RAM, Disk, Battery, and AI/dev processes).
    /// With the app's pin (`helper_sha256`) the pinned host helper answers with typed JSON; without
    /// it, or when it is absent, older or replaced, the shell script runs as before.
    pub async fn fetch_host_telemetry(
        &self,
        helper_sha256: Option<String>,
    ) -> Result<crate::telemetry::HostTelemetrySnapshot, SshCoreError> {
        if let Some(pin) = &helper_sha256 {
            crate::usage::validate_sha256(pin)?;
            let out = self
                .execute_command_on_active_session(
                    crate::telemetry::build_helper_telemetry_command(pin),
                )
                .await?;
            if let Some(snap) = crate::telemetry::parse_helper_telemetry(&out) {
                return Ok(snap);
            }
        }
        let raw_output = self
            .execute_command_on_active_session(
                crate::telemetry::TELEMETRY_COLLECTION_SCRIPT.to_string(),
            )
            .await?;
        Ok(crate::telemetry::parse_host_telemetry(raw_output))
    }

    /// TCP ports the host listens on, with the program behind each (`ss`, or `lsof` on macOS).
    pub async fn fetch_listening_ports(
        &self,
    ) -> Result<Vec<crate::ports::ListeningPort>, SshCoreError> {
        let out = self
            .execute_command_on_active_session(crate::ports::LISTENING_PORTS_COMMAND.to_string())
            .await?;
        Ok(crate::ports::parse_listening_ports(&out))
    }

    /// Safely terminates a remote process by PID (`SIGTERM` or `SIGKILL`), rejecting reserved PIDs <= 1.
    /// With the app's pin the pinned host helper does it and refuses the dangerous targets (this
    /// connection's own processes, other accounts, the tmux server, a reused pid when `started` is
    /// given); without it the plain `kill` runs as before.
    pub async fn kill_remote_process(
        &self,
        pid: u32,
        force: bool,
        started: Option<u64>,
        helper_sha256: Option<String>,
    ) -> Result<(), SshCoreError> {
        let safe_pid = crate::telemetry::validate_kill_pid(pid)?;
        if let Some(pin) = &helper_sha256 {
            crate::usage::validate_sha256(pin)?;
            let out = self
                .execute_command_on_active_session(crate::telemetry::build_helper_kill_command(
                    pin, safe_pid, force, started,
                ))
                .await?;
            match crate::telemetry::parse_helper_kill(&out) {
                Some(Ok(())) => return Ok(()),
                Some(Err(reason)) => return Err(SshCoreError::IoError { reason }),
                None => {}
            }
        }
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
    /// With the app's pin the pinned host helper lists it (typed, policy checked on the host); an
    /// absent, older or replaced helper falls back to the `ls -la` script.
    pub async fn list_remote_directory(
        &self,
        path: Option<String>,
        include_hidden: bool,
        helper_sha256: Option<String>,
    ) -> Result<crate::file_browser::RemoteDirectoryListing, SshCoreError> {
        let session_name = read_ok(&self.config).session_name.clone();
        if let Some(pin) = &helper_sha256 {
            crate::usage::validate_sha256(pin)?;
            let cmd = crate::file_browser::build_helper_ls_command(
                pin,
                path.as_deref(),
                &session_name,
                include_hidden,
            )?;
            let out = self.execute_command_on_active_session(cmd).await?;
            match crate::file_browser::parse_helper_listing(&out, include_hidden) {
                Some(Ok(listing)) => {
                    // Belt and braces: the app's own validator agrees with the helper's policy.
                    crate::file_browser::validate_remote_dir_path(&listing.current_path)?;
                    return Ok(listing);
                }
                Some(Err(reason)) => return Err(SshCoreError::InvalidInput { reason }),
                None => {}
            }
        }
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
        // The common case (server up) is answered by a plain `sh` curl in a moment; the login
        // shell and `ollama --version` of the full probe can take seconds on a heavy profile.
        if let Ok(Ok(quick)) = tokio::time::timeout(
            Duration::from_secs(4),
            self.execute_command_on_active_session(crate::ai::build_ollama_quick_probe_command()),
        )
        .await
        {
            let status = crate::ai::parse_ollama_probe(quick);
            if status.running {
                return Ok(status);
            }
        }
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            self.execute_command_on_active_session(crate::ai::build_ollama_probe_command()),
        )
        .await
        .map_err(|_| SshCoreError::Timeout)??;
        Ok(crate::ai::parse_ollama_probe(out))
    }
}
