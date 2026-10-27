use super::*;
use crate::agents::{AgentKind, HookPreset};
use crate::container_runtime::{parse_runtime_probe, runtime_probe_command, ContainerRuntime};
use crate::sandbox::{
    egress_allow_command, egress_allowed_command, egress_log_command, egress_remove_command,
    parse_allowed_hosts, parse_denied_hosts, parse_review_files, plan_sandbox,
    review_files_command, sandbox_hook_config_dir, SandboxAgent, SandboxBranchReview,
    SandboxEngine, SandboxProfile, SANDBOX_HOOK_DIR,
};

/// Image builds and clones take minutes on a cold engine.
const BUILD_SECS: u64 = 900;
const STEP_SECS: u64 = 120;

fn agent_kind(a: SandboxAgent) -> AgentKind {
    match a {
        SandboxAgent::ClaudeCode => AgentKind::ClaudeCode,
        SandboxAgent::Antigravity => AgentKind::Antigravity,
        SandboxAgent::Codex => AgentKind::Codex,
        SandboxAgent::Grok => AgentKind::Grok,
    }
}

/// A step's output ends with `SBX_OK` when the script reached its end; anything else is a failure
/// whose last lines (cleaned) are the reason.
fn step_failure(out: &str, what: &str) -> SshCoreError {
    let tail: String = out
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    SshCoreError::IoError {
        reason: format!(
            "{what} failed: {}",
            crate::approval_queue::display_safe(&tail.chars().take(400).collect::<String>())
        ),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// The sandbox engines installed on the server (Docker, Podman), in detection order. The
    /// app asks when both exist, uses the one that does, and offers install or an unprotected
    /// start when none does (spec 62 §7.1).
    pub async fn detect_sandbox_engines(&self) -> Result<Vec<SandboxEngine>, SshCoreError> {
        let out = self
            .execute_command_on_active_session(runtime_probe_command())
            .await?;
        Ok(parse_runtime_probe(out)
            .into_iter()
            .filter_map(|r| match r {
                ContainerRuntime::Docker => Some(SandboxEngine::Docker),
                ContainerRuntime::Podman => Some(SandboxEngine::Podman),
                _ => None,
            })
            .collect())
    }

    /// Builds the images, creates the volume, networks and proxy, writes the hooks into the
    /// sandbox home and clones the project's committed content. Returns the command that runs
    /// the agent in the tab. Idempotent: safe to repeat after a failure.
    pub async fn prepare_sandbox(
        &self,
        profile: SandboxProfile,
        hook_preset: HookPreset,
    ) -> Result<String, SshCoreError> {
        let plan = plan_sandbox(&profile)?;
        let mark = " && echo SBX_OK";
        let mut steps: Vec<(&str, String, u64)> = vec![(
            "Building the agent image",
            plan.build_command.clone(),
            BUILD_SECS,
        )];
        if !plan.proxy_build_command.is_empty() {
            steps.push((
                "Building the proxy image",
                plan.proxy_build_command.clone(),
                BUILD_SECS,
            ));
        }
        for c in &plan.setup_commands {
            steps.push(("Creating the sandbox", c.clone(), STEP_SECS));
        }
        for (what, cmd, secs) in steps {
            let out = self
                .exec_with_timeout(&format!("({cmd}){mark}"), Duration::from_secs(secs))
                .await?;
            if !out.trim_end().ends_with("SBX_OK") {
                return Err(step_failure(&out, what));
            }
        }
        if !plan.login_command.is_empty() {
            self.exec_with_timeout(
                &format!("({}){mark}", plan.login_command),
                Duration::from_secs(STEP_SECS),
            )
            .await
            .and_then(|out| {
                if out.trim_end().ends_with("SBX_OK") {
                    Ok(())
                } else {
                    Err(step_failure(&out, "Copying the agent login"))
                }
            })?;
        }
        if !profile.auto_approve {
            self.install_sandbox_hooks(&profile, hook_preset).await?;
        }
        let out = self
            .exec_with_timeout(
                &format!("({}){mark}", plan.clone_command),
                Duration::from_secs(BUILD_SECS),
            )
            .await?;
        if !out.trim_end().ends_with("SBX_OK") {
            return Err(step_failure(&out, "Copying the project"));
        }
        Ok(plan.run_command)
    }

    /// Brings the sandbox's branches home under the `sandbox/<id>/` prefix (bundle fetch, nothing else crosses).
    pub async fn fetch_sandbox_branches(
        &self,
        profile: SandboxProfile,
    ) -> Result<Vec<String>, SshCoreError> {
        let plan = plan_sandbox(&profile)?;
        let out = self
            .exec_with_timeout(
                &format!("({}) 2>&1 && echo SBX_OK", plan.fetch_command),
                Duration::from_secs(BUILD_SECS),
            )
            .await?;
        if !out.trim_end().ends_with("SBX_OK") {
            return Err(step_failure(&out, "Fetching the sandbox branches"));
        }
        Ok(out
            .lines()
            .filter_map(|l| l.split_whitespace().last())
            .filter(|r| r.starts_with(&format!("sandbox/{}/", profile.sandbox_id)))
            .map(str::to_string)
            .collect())
    }

    /// For each branch brought back: how many files it changed and which of them the host would
    /// run later (git hooks, build files, CI, agent configs). The branches are only read.
    pub async fn review_sandbox_branches(
        &self,
        profile: SandboxProfile,
        branches: Vec<String>,
    ) -> Result<Vec<SandboxBranchReview>, SshCoreError> {
        let mut out = Vec::new();
        for b in branches.into_iter().take(20) {
            let cmd = review_files_command(&profile, &b)?;
            let text = self
                .exec_with_timeout(&format!("({cmd}) 2>&1"), Duration::from_secs(STEP_SECS))
                .await?;
            out.push(parse_review_files(&b, &text));
        }
        Ok(out)
    }

    /// The hosts the sandbox's proxy has refused lately, for the owner to allow or ignore.
    pub async fn sandbox_blocked_hosts(
        &self,
        profile: SandboxProfile,
    ) -> Result<Vec<String>, SshCoreError> {
        let allowed = self.sandbox_allowed_hosts(profile.clone()).await?;
        let log = self
            .exec_with_timeout(
                &egress_log_command(&profile)?,
                Duration::from_secs(STEP_SECS),
            )
            .await?;
        Ok(parse_denied_hosts(&log, &allowed))
    }

    /// The hosts the sandbox's proxy lets through right now.
    pub async fn sandbox_allowed_hosts(
        &self,
        profile: SandboxProfile,
    ) -> Result<Vec<String>, SshCoreError> {
        let out = self
            .exec_with_timeout(
                &egress_allowed_command(&profile)?,
                Duration::from_secs(STEP_SECS),
            )
            .await?;
        Ok(parse_allowed_hosts(&out))
    }

    /// Lets one more host through (the owner's tap) and reloads the running proxy.
    pub async fn allow_sandbox_host(
        &self,
        profile: SandboxProfile,
        host: String,
    ) -> Result<(), SshCoreError> {
        let cmd = egress_allow_command(&profile, &host)?;
        let out = self
            .exec_with_timeout(
                &format!("({cmd}) 2>&1 && echo SBX_OK"),
                Duration::from_secs(STEP_SECS),
            )
            .await?;
        if !out.trim_end().ends_with("SBX_OK") {
            return Err(step_failure(&out, "Allowing the host"));
        }
        Ok(())
    }

    /// Takes a host out of the sandbox's allow-list.
    pub async fn disallow_sandbox_host(
        &self,
        profile: SandboxProfile,
        host: String,
    ) -> Result<(), SshCoreError> {
        let cmd = egress_remove_command(&profile, &host)?;
        let out = self
            .exec_with_timeout(
                &format!("({cmd}) 2>&1 && echo SBX_OK"),
                Duration::from_secs(STEP_SECS),
            )
            .await?;
        if !out.trim_end().ends_with("SBX_OK") {
            return Err(step_failure(&out, "Removing the host"));
        }
        Ok(())
    }

    /// Stops and removes the containers, networks and work volume (the sandbox home with the
    /// agent's logins is kept).
    pub async fn remove_sandbox(&self, profile: SandboxProfile) -> Result<(), SshCoreError> {
        let plan = plan_sandbox(&profile)?;
        for c in plan.cleanup_commands {
            self.exec_with_timeout(&c, Duration::from_secs(STEP_SECS))
                .await?;
        }
        Ok(())
    }
}

impl SshSessionHandle {
    /// Writes the phone-approval hooks into the sandbox home (never into the host's agent configs):
    /// the container reads them from `/home/agent`, the script from `/usr/local/bin`.
    async fn install_sandbox_hooks(
        &self,
        profile: &SandboxProfile,
        preset: HookPreset,
    ) -> Result<(), SshCoreError> {
        let agent = agent_kind(profile.agent);
        let dir = sandbox_hook_config_dir(profile)?;
        let path = crate::agents::agent_hooks_config_path(agent, Some(dir))?;
        let planned =
            crate::agents::plan_agent_hooks(agent, preset, Some(SANDBOX_HOOK_DIR.to_string()))?;
        let existing = self.read_remote_config(&path).await?;
        let merged = crate::agents::merge_planned_hooks(agent, existing.clone(), planned)?;
        self.write_remote_config(&path, &existing, &merged).await
    }
}

/// The minimal profile the egress commands need (they only use the engine and the sandbox id).
fn egress_profile(engine: SandboxEngine, sandbox_id: String) -> SandboxProfile {
    SandboxProfile {
        engine,
        agent: SandboxAgent::ClaudeCode,
        sandbox_id,
        project_folder: "~/p".to_string(),
        network: crate::sandbox::SandboxNetwork::Online,
        allow_hosts: Vec::new(),
        cpus: 1,
        memory_mb: 1024,
        pids: 64,
        auto_approve: false,
        base_image: crate::sandbox::sandbox_default_base_image(),
        copy_login: false,
        claude_config_dir: String::new(),
    }
}

/// The hosts a sandbox's proxy refused lately, from one short SSH exec and no session: for the
/// push notification extension, which has a few seconds and little memory. The allow-list and the
/// proxy log come back in the same call. Nothing is written on the host.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sandbox_blocked_hosts_for(
    config: crate::config::RemoteServerConfig,
    engine: SandboxEngine,
    sandbox_id: String,
) -> Result<Vec<String>, SshCoreError> {
    let profile = egress_profile(engine, sandbox_id);
    let command = format!(
        "({}); echo {SEPARATOR}; ({})",
        egress_allowed_command(&profile)?,
        egress_log_command(&profile)?
    );
    let task = crate::session::RUNTIME.spawn(async move {
        crate::session::execute_remote_command_internal(&config, &command).await
    });
    let out = task.await.map_err(|e| SshCoreError::ConnectionError {
        reason: format!("Task join error: {e}"),
    })??;
    let (filter, log) = out.split_once(SEPARATOR).unwrap_or(("", &out));
    Ok(parse_denied_hosts(log, &parse_allowed_hosts(filter)))
}

const SEPARATOR: &str = "SBX_SEP_7f3a";

#[cfg(test)]
mod egress_tests {
    use super::*;

    #[test]
    fn the_light_profile_passes_validation_and_names_the_proxy() {
        let p = egress_profile(SandboxEngine::Podman, "sbx-ab12cd34".into());
        let cmd = egress_log_command(&p).unwrap();
        assert!(
            cmd.contains("podman logs") && cmd.contains("ssh-app-sbx-sbx-ab12cd34"),
            "{cmd}"
        );
        assert!(egress_allowed_command(&p).is_ok());
        assert!(
            egress_log_command(&egress_profile(SandboxEngine::Docker, "bad id;x".into())).is_err()
        );
    }
}
