use super::*;
use crate::docker::{self, DockerAction, DockerContainerStats, DockerContainersSnapshot};

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Containers on the host through its `docker` CLI. Trouble (not installed, daemon down, no
    /// permission) is a snapshot with `docker_available == false` and a message, not an error.
    pub async fn fetch_docker_containers(&self) -> Result<DockerContainersSnapshot, SshCoreError> {
        let out = self
            .execute_command_on_active_session(docker::docker_list_command())
            .await?;
        Ok(docker::parse_docker_list(out))
    }

    /// Memory and CPU of one container: a separate, slower call (`docker stats` takes 1-2 s).
    pub async fn fetch_docker_stats(
        &self,
        container_id: String,
    ) -> Result<DockerContainerStats, SshCoreError> {
        let cmd = docker::docker_stats_command(container_id)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(docker::parse_docker_stats(out))
    }

    /// Starts, stops or restarts a container. The app asks the user first (it may be a production
    /// database); the core refuses an identifier that is not a plain name or id.
    pub async fn docker_container_action(
        &self,
        action: DockerAction,
        container_id: String,
    ) -> Result<(), SshCoreError> {
        let cmd = docker::docker_action_command(action, container_id)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        docker::parse_docker_action(out)
    }

    /// The last lines of a container's log (at most 1000, 200 when 0), cleaned of escape
    /// sequences and cut at 1 MB.
    pub async fn fetch_docker_logs(
        &self,
        container_id: String,
        tail_lines: u32,
    ) -> Result<String, SshCoreError> {
        let cmd = docker::docker_logs_command(container_id, tail_lines)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(docker::clean_docker_logs(out))
    }
}
