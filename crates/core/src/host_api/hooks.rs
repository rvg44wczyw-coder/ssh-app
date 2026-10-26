use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
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
        self.install_agent_hooks_at(agent, &config_path, hook_dir, preset)
            .await
    }

    /// The Hooks screen of this server (spec 61): the agents the pinned host helper finds, their
    /// user-level hook state and the projects (running panes, agent histories, `folders`).
    pub async fn agent_hooks_overview(
        &self,
        helper_sha256: String,
        folders: Vec<String>,
        claude_config_dir: Option<String>,
    ) -> Result<Vec<crate::agents::AgentHooksOverview>, SshCoreError> {
        let cmd = crate::agents::build_hooks_overview_command(
            &helper_sha256,
            &folders,
            claude_config_dir.as_deref(),
        )?;
        let out = self.execute_command_on_active_session(cmd).await?;
        crate::agents::parse_hooks_overview(&out)
    }

    /// `agent_hooks_status` for a scope (spec 61).
    pub async fn agent_hooks_status_in(
        &self,
        agent: crate::agents::AgentKind,
        scope: crate::agents::HookScope,
        hook_dir: Option<String>,
    ) -> Result<crate::agents::AgentHooksStatus, SshCoreError> {
        let config_path = crate::agents::agent_hooks_target_path(agent, scope)?;
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

    /// `preview_agent_hooks` for a scope.
    pub async fn preview_agent_hooks_in(
        &self,
        agent: crate::agents::AgentKind,
        scope: crate::agents::HookScope,
        hook_dir: Option<String>,
        preset: crate::agents::HookPreset,
    ) -> Result<String, SshCoreError> {
        let config_path = crate::agents::agent_hooks_target_path(agent, scope)?;
        let planned = crate::agents::plan_agent_hooks(agent, preset, hook_dir)?;
        let existing = self.read_remote_config(&config_path).await?;
        let merged = crate::agents::merge_planned_hooks(agent, existing, planned)?;
        crate::agents::hooks_section_of(agent, merged)
    }

    /// `installed_agent_hooks` for a scope.
    pub async fn installed_agent_hooks_in(
        &self,
        agent: crate::agents::AgentKind,
        scope: crate::agents::HookScope,
    ) -> Result<Vec<crate::agents::PlannedHook>, SshCoreError> {
        let config_path = crate::agents::agent_hooks_target_path(agent, scope)?;
        let existing = self.read_remote_config(&config_path).await?;
        Ok(crate::agents::installed_hook_entries(agent, existing))
    }

    /// `uninstall_agent_hooks` for a scope.
    pub async fn uninstall_agent_hooks_in(
        &self,
        agent: crate::agents::AgentKind,
        scope: crate::agents::HookScope,
    ) -> Result<(), SshCoreError> {
        let config_path = crate::agents::agent_hooks_target_path(agent, scope)?;
        let existing = self.read_remote_config(&config_path).await?;
        if existing.trim().is_empty() {
            return Ok(());
        }
        let cleaned = crate::agents::remove_agent_hooks(agent, existing.clone())?;
        self.write_remote_config(&config_path, &existing, &cleaned)
            .await
    }

    /// `install_agent_hooks_with_preset` for a scope. A project install is refused while the
    /// user-level hooks of the agent are present, unless `disable_user_level` is set: then those
    /// are removed first (the caller asked the user to confirm that), and only ours.
    pub async fn install_agent_hooks_with_preset_in(
        &self,
        agent: crate::agents::AgentKind,
        scope: crate::agents::HookScope,
        hook_dir: Option<String>,
        preset: crate::agents::HookPreset,
        disable_user_level: bool,
    ) -> Result<(), SshCoreError> {
        match scope {
            crate::agents::HookScope::User { config_dir } => {
                self.install_agent_hooks_with_preset(agent, config_dir, hook_dir, preset)
                    .await
            }
            crate::agents::HookScope::Project {
                folder,
                user_config_dir,
            } => {
                let project_path = crate::agents::agent_project_hooks_path(agent, folder)?;
                let user_path = crate::agents::agent_hooks_config_path(agent, user_config_dir)?;
                let user_level = self.read_remote_config(&user_path).await?;
                if disable_user_level
                    && crate::agents::agent_hooks_installed(agent, user_level.clone())
                {
                    let cleaned = crate::agents::remove_agent_hooks(agent, user_level.clone())?;
                    self.write_remote_config(&user_path, &user_level, &cleaned)
                        .await?;
                } else {
                    crate::agents::check_project_install(agent, user_level)?;
                }
                self.install_agent_hooks_at(agent, &project_path, hook_dir, preset)
                    .await
            }
        }
    }
}

impl SshSessionHandle {
    /// The install itself, into the config file at `config_path`.
    async fn install_agent_hooks_at(
        &self,
        agent: crate::agents::AgentKind,
        config_path: &str,
        hook_dir: Option<String>,
        preset: crate::agents::HookPreset,
    ) -> Result<(), SshCoreError> {
        let hook_dir = hook_dir.unwrap_or_else(|| crate::agents::DEFAULT_HOOK_DIR.to_string());

        let uses_script = crate::agents::hook_preset_uses_script(preset.clone());
        let preset_for_install = preset.clone();
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

        let existing = self.read_remote_config(config_path).await?;
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
        let uses_extra = crate::agents::hook_preset_uses_extra(preset_for_install.clone());
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
        if uses_extra {
            self.execute_command_on_active_session(format!(
                "umask 077; {}",
                crate::agents::build_write_file_command(
                    &crate::agents::remote_extra_script_path(&hook_dir),
                    &crate::agents::agent_hook_extra_script(),
                    "700"
                )
            ))
            .await?;
        }
        self.write_remote_config(config_path, &existing, &merged)
            .await
    }
}
