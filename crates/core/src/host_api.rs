//! Host-side features of an active session: transcripts, history, telemetry, files, agent hooks,
//! permissions and approvals. Every method is a thin, validated command on top of the session's
//! exec primitive (`SshSessionHandle::exec_with_timeout`); the pure parts live in their own modules.

use crate::error::SshCoreError;
use crate::session::{read_ok, SshSessionHandle};
use crate::transport::DEFAULT_EXEC_TIMEOUT;
use std::time::Duration;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Queries Ollama running locally on the remote host (http://127.0.0.1:11434/api/generate)
    /// through the active SSH session and returns the generated command suggestion.
    pub async fn query_host_ollama(
        &self,
        model: String,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<crate::ai::AiCommandSuggestion, SshCoreError> {
        self.query_ollama_on_port(11434, model, system_prompt, user_prompt)
            .await
    }

    /// One turn of a free-form chat with the host's Ollama: the whole history goes in, the
    /// assistant's reply comes back as plain text. Dropping the call stops the request on the host.
    pub async fn chat_host_ollama(
        &self,
        model: String,
        system_prompt: String,
        messages: Vec<crate::ai::ChatMessage>,
    ) -> Result<String, SshCoreError> {
        self.chat_ollama_on_port(11434, model, system_prompt, messages)
            .await
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

        // The host resolves symlinks first and reports the real path; it is checked against the
        // same blocklist before anything it returned is used (a link in an allowed folder must
        // not be a way to read `~/.ssh`).
        let p = crate::tmux::shell_quote(&remote_path);
        let cmd = format!(
            "r=$(realpath -- {p} 2>/dev/null || readlink -f -- {p} 2>/dev/null) && [ -f \"$r\" ] && \
             {{ printf 'TP_PATH=%s\\n' \"$r\"; tail -n 500 -- \"$r\"; }}"
        );
        let raw_output = self.execute_command_on_active_session(cmd).await?;
        let (first, rest) = raw_output
            .split_once('\n')
            .unwrap_or((raw_output.as_str(), ""));
        let resolved =
            first
                .strip_prefix("TP_PATH=")
                .ok_or_else(|| SshCoreError::TranscriptError {
                    reason: "Transcript file not found".to_string(),
                })?;
        crate::transcript::validate_transcript_path(resolved).map_err(|e| {
            SshCoreError::TranscriptError {
                reason: e.to_string(),
            }
        })?;

        crate::transcript::parse_transcript_jsonl(rest.to_string()).map_err(|e| {
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
            installed: crate::agents::installed_hook_entries(agent, existing.clone()),
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

    /// Reads what is in effect for this session: its folder (tmux pane), the profile's rules and the
    /// folder's `.claude/settings.json` / `settings.local.json` (read-only), plus suggestions.
    /// Only Claude Code has readable rule files; other agents get an empty report.
    pub async fn inspect_session_permissions(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
    ) -> Result<crate::permissions::SessionPermissionsReport, SshCoreError> {
        let explicit = config_dir
            .as_deref()
            .map(str::trim)
            .is_some_and(|d| !d.is_empty());
        let file_expr = if agent == crate::agents::AgentKind::ClaudeCode && !explicit {
            None
        } else {
            let path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
            Some(crate::agents::remote_path_expr(&path))
        };
        let session = self
            .config
            .read()
            .map_err(|_| SshCoreError::IoError {
                reason: "session state unavailable".to_string(),
            })?
            .session_name
            .clone();
        crate::tmux::validate_session_name(&session)?;
        let tag = crate::permissions::new_inspect_tag();
        let cmd = crate::permissions::build_inspect_permissions_command(
            &session,
            file_expr.as_deref(),
            &tag,
        );
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::permissions::parse_inspect_permissions_output(
            &out, &tag,
        ))
    }

    /// Like `read_agent_permissions`, but rules the editor cannot represent (e.g. a line break
    /// inside a rule) are returned separately instead of failing the read.
    pub async fn read_agent_permissions_checked(
        &self,
        agent: crate::agents::AgentKind,
        config_dir: Option<String>,
    ) -> Result<crate::permissions::CheckedPermissions, SshCoreError> {
        let config_path = crate::agents::agent_hooks_config_path(agent, config_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        crate::permissions::permissions_from_settings_checked(agent, existing)
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

    /// The latest commits of the repository the tab's pane is in (newest first).
    pub async fn fetch_git_log(
        &self,
        session_name: String,
        limit: u32,
    ) -> Result<crate::gitlog::GitLogResult, SshCoreError> {
        let cmd = crate::gitlog::build_git_log_command(&session_name, limit)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitlog::parse_git_log_output(&out, limit))
    }

    /// The patch of one commit, shaped like a diff result for the diff viewer.
    pub async fn fetch_git_show(
        &self,
        session_name: String,
        hash: String,
    ) -> Result<crate::gitdiff::GitDiffResult, SshCoreError> {
        let cmd = crate::gitlog::build_git_show_command(&session_name, &hash)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::gitlog::parse_git_show_output(&out))
    }

    /// Whether the tab's pane runs an AI CLI or a plain shell (see `pane_context`). A host error
    /// is "unknown", which callers treat like a shell.
    pub async fn detect_pane_context(
        &self,
        session_name: String,
    ) -> Result<crate::pane_context::PaneContext, SshCoreError> {
        let cmd = crate::pane_context::build_pane_context_command(&session_name)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::pane_context::parse_pane_context(&out))
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

    /// Like `list_pending_approvals`, but the host holds the call (up to ~20 s) until the pending
    /// set differs from `known_ids`, so changes arrive at once and an idle phone stays quiet.
    /// Hosts whose hook script predates `ctl wait` answer an error at once; that falls back to
    /// a plain list, and the caller paces itself on how fast this returns.
    pub async fn wait_pending_approvals(
        &self,
        known_ids: Vec<String>,
    ) -> Result<crate::approval_queue::ApprovalList, SshCoreError> {
        use crate::approval_queue::{build_wait_command, parse_approval_list};
        let out = self
            .execute_command_on_active_session(build_wait_command(&known_ids)?)
            .await?;
        let list = parse_approval_list(&out);
        if list.host_time_sec == 0 {
            return self.list_pending_approvals().await;
        }
        Ok(list)
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

    /// The host guard (`~/.ssh-app/guard.json`); a missing file is an empty guard. A file the app
    /// cannot understand is an error, so it is never overwritten by accident.
    pub async fn read_host_guard(&self) -> Result<crate::host_guard::HostGuard, SshCoreError> {
        let text = self
            .read_remote_config(crate::host_guard::GUARD_FILE)
            .await?;
        crate::host_guard::host_guard_from_json(text)
    }

    /// Validates and writes the host guard (folder `0700`, file `0600`).
    pub async fn apply_host_guard(
        &self,
        guard: crate::host_guard::HostGuard,
    ) -> Result<(), SshCoreError> {
        let json = crate::host_guard::host_guard_to_json(guard)?;
        let path = crate::agents::remote_path_expr(crate::host_guard::GUARD_FILE);
        self.execute_command_on_active_session(format!(
            "umask 077; mkdir -p \"$HOME/.ssh-app\" && chmod 700 \"$HOME/.ssh-app\"; {}",
            crate::agents::build_write_file_command(&path, &json, "600")
        ))
        .await?;
        Ok(())
    }

    /// Agent sessions the hook script registered (needs the "Register sessions" template).
    pub async fn list_agent_sessions(
        &self,
    ) -> Result<Vec<crate::host_guard::AgentSessionInfo>, SshCoreError> {
        let out = self
            .execute_command_on_active_session(crate::host_guard::SESSIONS_COMMAND.to_string())
            .await?;
        Ok(crate::host_guard::parse_agent_sessions(out))
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

impl SshSessionHandle {
    /// `chat_host_ollama` against another port (tests run a fake server).
    #[doc(hidden)]
    pub async fn chat_ollama_on_port(
        &self,
        port: u16,
        model: String,
        system_prompt: String,
        messages: Vec<crate::ai::ChatMessage>,
    ) -> Result<String, SshCoreError> {
        let payload = crate::ai::build_ollama_chat_payload(&model, &system_prompt, &messages)?;
        let curl_cmd = crate::hostprobe::cancellable_sh(&format!(
            "curl -s --max-time 290 -X POST http://127.0.0.1:{port}/api/chat -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload)
        ));
        let raw = self
            .exec_with_timeout(&curl_cmd, Duration::from_secs(300))
            .await?;
        crate::ai::parse_ollama_chat_response(&raw)
    }

    /// `query_host_ollama` against another port (tests run a fake server; the product uses 11434).
    #[doc(hidden)]
    pub async fn query_ollama_on_port(
        &self,
        port: u16,
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

        let curl_cmd = crate::hostprobe::cancellable_sh(&format!(
            "curl -s --max-time 290 -X POST http://127.0.0.1:{port}/api/generate -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload_str)
        ));

        // Local models can be slow, but not forever.
        let raw_response = self
            .exec_with_timeout(&curl_cmd, Duration::from_secs(300))
            .await?;
        let generated_text = crate::ai::parse_ollama_generate_response(&raw_response)?;
        crate::ai::parse_ai_response(&generated_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
