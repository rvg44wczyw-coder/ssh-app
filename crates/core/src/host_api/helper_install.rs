use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Installs the helper the app ships (`~/.ssh-app/bin/ssh-app-host`, mode 700). The digest is
    /// checked here, again on the host before the file is moved into place, and the result is
    /// refused when it differs. Nothing is executed during the install.
    pub async fn install_host_helper(
        &self,
        bytes: Vec<u8>,
        sha256: String,
    ) -> Result<(), SshCoreError> {
        let cmds = crate::usage::build_helper_install_commands(&bytes, &sha256)?;
        let last = cmds.len() - 1;
        for (i, cmd) in cmds.into_iter().enumerate() {
            let out = self.execute_command_on_active_session(cmd).await?;
            if i == last && !out.contains("HELPER_INSTALLED") {
                return Err(SshCoreError::IoError {
                    reason: "The host helper did not install (digest mismatch on the host)"
                        .to_string(),
                });
            }
        }
        Ok(())
    }

    /// Keeps the installed hook script in step with the helper: its pin is inside the script, and
    /// a stale pin sends every hook to the Python code. Replaces the script only when it is
    /// ssh-app's own; any other file there is left alone unless `overwrite_foreign` (the user
    /// said yes).
    pub async fn refresh_hook_script(
        &self,
        overwrite_foreign: bool,
    ) -> Result<crate::agent_hooks::HookScriptRefresh, SshCoreError> {
        use crate::agent_hooks::{plan_hook_script_refresh, HookScriptRefresh};
        let path = crate::agents::remote_script_path(crate::agents::DEFAULT_HOOK_DIR);
        let existing = self.read_remote_config(&path).await?;
        let new = crate::agents::agent_hook_script();
        let plan = plan_hook_script_refresh(&existing, &new, overwrite_foreign);
        if plan == HookScriptRefresh::Refreshed {
            self.execute_command_on_active_session(format!(
                "umask 077; {}",
                crate::agents::build_write_file_command(&path, &new, "700")
            ))
            .await?;
        }
        Ok(plan)
    }

    /// Which code the hooks run on this host: the Rust helper (the script's pin is the helper this
    /// build ships), the Python code, or a file that is not ssh-app's.
    pub async fn hook_script_state(
        &self,
    ) -> Result<crate::agent_hooks::HookScriptState, SshCoreError> {
        let path = crate::agents::remote_script_path(crate::agents::DEFAULT_HOOK_DIR);
        let existing = self.read_remote_config(&path).await?;
        let pin = crate::usage::bundled_host_helper_sha256();
        Ok(crate::agent_hooks::classify_hook_script(
            &existing,
            pin.as_deref(),
        ))
    }

    /// Which kind of host this is (macOS / Linux / WSL2 and the CPU), from one short probe.
    pub async fn detect_host_target(&self) -> Result<crate::hosttarget::HostTarget, SshCoreError> {
        let out = self
            .execute_command_on_active_session(crate::hostprobe::login_sh_script(
                crate::hosttarget::HOST_TARGET_SCRIPT,
                &[],
                10,
            ))
            .await?;
        let target = crate::hosttarget::parse_host_target(out);
        // cmd.exe / PowerShell answered the POSIX probe with an error: ask them the Windows way.
        if matches!(&target, crate::hosttarget::HostTarget::Unsupported { reason } if reason.starts_with("Windows without WSL2"))
        {
            if let Ok(out) = self
                .execute_command_on_active_session(
                    crate::hosttarget::WINDOWS_TARGET_SCRIPT.to_string(),
                )
                .await
            {
                if let Some(windows) = crate::hosttarget::parse_windows_target(&out) {
                    return Ok(windows);
                }
            }
        }
        Ok(target)
    }

    /// Installs the helper build that matches `target` (digest checked on the host as always).
    /// Fails for an unsupported system or a build this app does not embed.
    pub async fn install_bundled_host_helper_for(
        &self,
        target: crate::hosttarget::HostTarget,
    ) -> Result<(), SshCoreError> {
        let triple = crate::hosttarget::require_supported(&target)?;
        let bytes =
            crate::usage::bundled_helper_for(&target).ok_or(SshCoreError::InvalidInput {
                reason: format!("This build of the app has no host helper for {triple}"),
            })?;
        let sha = crate::usage::bundled_host_helper_sha256_for(target).ok_or(
            SshCoreError::InvalidInput {
                reason: "This build has no host helper for that system".to_string(),
            },
        )?;
        self.install_host_helper(bytes.to_vec(), sha).await
    }

    /// Installs the helper embedded in this build (see `bundled_host_helper_sha256`).
    pub async fn install_bundled_host_helper(&self) -> Result<(), SshCoreError> {
        let bytes = crate::usage::bundled_host_helper();
        let sha = crate::usage::bundled_host_helper_sha256().ok_or(SshCoreError::InvalidInput {
            reason: "This build has no host helper embedded".to_string(),
        })?;
        self.install_host_helper(bytes.to_vec(), sha).await
    }

    /// Removes the wrapper and restores the user's previous status line.
    pub async fn uninstall_claude_statusline(
        &self,
        config_dir: Option<String>,
    ) -> Result<(), SshCoreError> {
        let path = crate::agents::agent_hooks_config_path(
            crate::agents::AgentKind::ClaudeCode,
            config_dir,
        )?;
        let existing = self.read_remote_config(&path).await?;
        if existing.trim().is_empty() {
            return Ok(());
        }
        let cleaned = crate::agents::remove_claude_statusline(existing.clone())?;
        self.write_remote_config(&path, &existing, &cleaned).await
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
    /// With the app's pin the pinned host helper reads it in one step (canonical path, policy, open
    /// without following links, regular files only); otherwise `realpath` + `head` as before.
    pub async fn read_remote_file_preview(
        &self,
        path: String,
        max_bytes: u32,
        helper_sha256: Option<String>,
    ) -> Result<crate::file_browser::RemoteFilePreview, SshCoreError> {
        let requested = crate::file_browser::validate_remote_file_read_path(&path)?;
        let capped_max = (max_bytes as usize).clamp(1024, 131_072);
        if let Some(pin) = &helper_sha256 {
            crate::usage::validate_sha256(pin)?;
            let cmd = crate::file_browser::build_helper_read_command(pin, &requested, capped_max);
            let out = self.execute_command_on_active_session(cmd).await?;
            match crate::file_browser::parse_helper_read(&out) {
                Some(Ok(preview)) => {
                    crate::file_browser::validate_remote_file_read_path(&preview.path)?;
                    return Ok(preview);
                }
                Some(Err(reason)) => return Err(SshCoreError::IoError { reason }),
                None => {}
            }
        }
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
