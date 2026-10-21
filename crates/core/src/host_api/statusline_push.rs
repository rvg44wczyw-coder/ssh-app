use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Whether the plan-limits status line wrapper is in the Claude profile's `settings.json`.
    pub async fn claude_statusline_status(
        &self,
        config_dir: Option<String>,
    ) -> Result<bool, SshCoreError> {
        let path = crate::agents::agent_hooks_config_path(
            crate::agents::AgentKind::ClaudeCode,
            config_dir,
        )?;
        let existing = self.read_remote_config(&path).await?;
        Ok(crate::agents::claude_statusline_installed(existing))
    }

    /// Dry run: the `statusLine` entry that would be written (nothing else from the file).
    pub async fn preview_claude_statusline(
        &self,
        config_dir: Option<String>,
        hook_dir: Option<String>,
    ) -> Result<String, SshCoreError> {
        let path = crate::agents::agent_hooks_config_path(
            crate::agents::AgentKind::ClaudeCode,
            config_dir,
        )?;
        let existing = self.read_remote_config(&path).await?;
        let merged = crate::agents::merge_claude_statusline(existing, hook_dir)?;
        let map = crate::agent_hooks::parse_object(&merged)?;
        let mut only = serde_json::Map::new();
        if let Some(v) = map.get("statusLine") {
            only.insert("statusLine".into(), v.clone());
        }
        crate::agent_hooks::render(only)
    }

    /// Installs the status line wrapper (and the host script it runs) into the Claude profile so
    /// the real 5-hour / 7-day plan limits can be shown. Keeps the user's own status line: the
    /// wrapper runs it. Backup, atomic write, refuses an unparsable config.
    pub async fn install_claude_statusline(
        &self,
        config_dir: Option<String>,
        hook_dir: Option<String>,
    ) -> Result<(), SshCoreError> {
        let path = crate::agents::agent_hooks_config_path(
            crate::agents::AgentKind::ClaudeCode,
            config_dir,
        )?;
        let hook_dir = hook_dir.unwrap_or_else(|| crate::agents::DEFAULT_HOOK_DIR.to_string());
        let existing = self.read_remote_config(&path).await?;
        let merged =
            crate::agents::merge_claude_statusline(existing.clone(), Some(hook_dir.clone()))?;
        let py = self
            .execute_command_on_active_session(
                "python3 --version >/dev/null 2>&1 && echo ok || echo missing".to_string(),
            )
            .await?;
        if py.trim() != "ok" {
            return Err(SshCoreError::IoError {
                reason: "python3 is required on the host to read the plan limits".to_string(),
            });
        }
        let script_path = crate::agents::remote_script_path(&hook_dir);
        let dir_expr = crate::agents::remote_path_expr(hook_dir.trim_end_matches('/'));
        let lock_default_parent = if hook_dir.trim_end_matches('/')
            == crate::agents::DEFAULT_HOOK_DIR
        {
            "mkdir -p \"$HOME/.ssh-app/run\" && chmod 700 \"$HOME/.ssh-app\" \"$HOME/.ssh-app/run\"; "
        } else {
            ""
        };
        // Script first, so the settings never point at a missing script.
        self.execute_command_on_active_session(format!(
            "umask 077; if [ ! -d {dir_expr} ]; then mkdir -p {dir_expr} && chmod 700 {dir_expr}; fi; {lock_default_parent}{}",
            crate::agents::build_write_file_command(
                &script_path,
                &crate::agents::agent_hook_script(),
                "700"
            )
        ))
        .await?;
        self.write_remote_config(&path, &existing, &merged).await
    }

    /// Whether the typed host helper is installed and is exactly the build the app pins.
    pub async fn host_helper_status(
        &self,
        expected_sha256: String,
    ) -> Result<crate::usage::HostHelperStatus, SshCoreError> {
        crate::usage::validate_sha256(&expected_sha256)?;
        let out = self
            .execute_command_on_active_session(crate::usage::build_helper_status_command(
                &expected_sha256,
            ))
            .await?;
        Ok(crate::usage::parse_helper_status(&out))
    }

    /// State of the agent sessions from the host agent (started on demand, lease 12 h); `None`
    /// when the pinned helper is not installed or the agent does not answer.
    pub async fn host_agent_state(
        &self,
        expected_sha256: String,
    ) -> Result<Option<crate::usage::HostAgentState>, SshCoreError> {
        crate::usage::validate_sha256(&expected_sha256)?;
        let out = self
            .execute_command_on_active_session(crate::usage::build_agent_state_command(
                &expected_sha256,
            ))
            .await?;
        Ok(crate::usage::parse_agent_state(&out))
    }

    /// Registers this phone for push on the host (feature 35): the helper stores the token and the
    /// relay's address and secret privately, the host agent sends the notifications.
    pub async fn host_push_register(
        &self,
        expected_sha256: String,
        token: String,
        env: String,
        relay_url: String,
        relay_secret: String,
    ) -> Result<crate::push::HostPushResult, SshCoreError> {
        let (cmd, input) = crate::push::build_register_command(
            &expected_sha256,
            &token,
            &env,
            &relay_url,
            &relay_secret,
        )?;
        let out = self.exec_with_stdin_text(&cmd, input.as_bytes()).await?;
        Ok(crate::push::parse_result(&out))
    }

    /// `None` when the helper is not usable.
    pub async fn host_push_status(
        &self,
        expected_sha256: String,
    ) -> Result<Option<crate::push::HostPushStatus>, SshCoreError> {
        let cmd = crate::push::build_push_command(&expected_sha256, "push-status")?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::push::parse_status(&out))
    }

    pub async fn host_push_unregister(
        &self,
        expected_sha256: String,
    ) -> Result<crate::push::HostPushResult, SshCoreError> {
        let cmd = crate::push::build_push_command(&expected_sha256, "push-unregister")?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::push::parse_result(&out))
    }

    /// Puts this phone's key on the host through the pinned helper (feature 44): the key and the
    /// device name travel on stdin, the helper appends to `authorized_keys` carefully and says
    /// whether anything changed.
    pub async fn pair_this_phone(
        &self,
        expected_sha256: String,
        device_name: String,
    ) -> Result<crate::onboarding::HostPairResult, SshCoreError> {
        let key = zeroize::Zeroizing::new(read_ok(&self.config).private_key_pem.clone());
        let public = crate::keys::derive_public_key((*key).clone())?;
        let (cmd, input) =
            crate::onboarding::build_pair_command(&expected_sha256, &public, &device_name)?;
        let out = self.exec_with_stdin_text(&cmd, input.as_bytes()).await?;
        Ok(crate::onboarding::parse_pair_result(&out))
    }

    /// Facts about the host from the pinned helper's `doctor`; `None` when the helper is not usable.
    pub async fn host_doctor(
        &self,
        expected_sha256: String,
    ) -> Result<Option<crate::onboarding::HostDoctor>, SshCoreError> {
        let cmd = crate::onboarding::build_doctor_command(&expected_sha256)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::onboarding::parse_host_doctor(&out))
    }
}
