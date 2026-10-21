use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
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
            .find(|i| i.id == id && i.kind == crate::approval_queue::ApprovalKind::Approval)
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
}
