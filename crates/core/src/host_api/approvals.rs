use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Whether the tab's pane runs an AI CLI or a plain shell (see `pane_context`). A host error
    /// is "unknown", which callers treat like a shell.
    pub async fn detect_pane_context(
        &self,
        session_name: String,
    ) -> Result<crate::pane_context::PaneContext, SshCoreError> {
        self.detect_pane_context_in(session_name, crate::multiplexer::MultiplexerKind::Tmux)
            .await
    }

    /// Like `detect_pane_context`, for a tab running in tmux or Zellij.
    pub async fn detect_pane_context_in(
        &self,
        session_name: String,
        kind: crate::multiplexer::MultiplexerKind,
    ) -> Result<crate::pane_context::PaneContext, SshCoreError> {
        let cmd = crate::pane_context::build_pane_context_command_in(kind, &session_name)?;
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
        let key = crate::key_store::load_pem(&read_ok(&self.config).key)?;
        let signature = sign_approval_sshsig(key.to_string(), message)?;
        let out = self
            .execute_command_on_active_session(build_decide_command(
                &item.id, approved, ts, &signature,
            )?)
            .await?;
        parse_ctl_result(&out)
    }

    /// Approves request `id` and, in the same signed step, trusts `category_ids` in the policy's
    /// project that contains the request's folder, for this agent session, for `trust_secs`
    /// (capped by the policy). Refused unless every category covers the approved command.
    pub async fn approve_with_trust(
        &self,
        id: String,
        category_ids: Vec<String>,
        trust_secs: u32,
    ) -> Result<(), SshCoreError> {
        use crate::approval_queue::{
            build_decide_command_with_trust, parse_ctl_result, sign_approval_sshsig,
        };
        let err = |reason: &str| SshCoreError::ApprovalVerificationError {
            reason: reason.to_string(),
        };
        let list = self.list_pending_approvals().await?;
        let item = list
            .items
            .iter()
            .find(|i| i.id == id && i.kind == crate::approval_queue::ApprovalKind::Approval)
            .filter(|i| i.expires_sec > list.host_time_sec)
            .ok_or_else(|| err("the request is gone or has expired"))?;
        if item.session.is_empty() {
            return Err(err("this agent has no session to bind a trust to"));
        }
        let guard = self.read_host_guard().await?;
        let offer = crate::auto_approval::auto_trust_offer(
            guard.auto.clone(),
            item.tool.clone(),
            item.command.clone(),
            item.cwd.clone(),
        )
        .ok_or_else(|| err("nothing can be trusted for this request"))?;
        if category_ids.is_empty() || category_ids.iter().any(|c| !offer.categories.contains(c)) {
            return Err(err("a category does not cover the approved command"));
        }
        let secs = u64::from(trust_secs.clamp(60, offer.max_secs));
        let trusted = crate::auto_approval::auto_trust_rules(category_ids)?;
        let ts = list.host_time_sec.max(item.timestamp_sec);
        let expires = ts + secs;
        let key = crate::key_store::load_pem(&read_ok(&self.config).key)?;
        let approval = crate::approval::create_canonical_signing_bytes(
            item.id.clone(),
            item.command_sha256.clone(),
            item.nonce.clone(),
            ts,
            true,
        );
        let signature = sign_approval_sshsig(key.to_string(), approval)?;
        let trust_bytes = crate::auto_approval::create_trust_signing_bytes(
            item.id.clone(),
            item.nonce.clone(),
            ts,
            expires,
            item.agent.clone(),
            item.session.clone(),
            offer.project_root.clone(),
            trusted.edits,
            trusted.rules.clone(),
        );
        let trust_signature = sign_approval_sshsig(key.to_string(), trust_bytes)?;
        let trust = serde_json::json!({
            "expires": expires, "agent": item.agent, "session": item.session,
            "root": offer.project_root, "edits": trusted.edits, "rules": trusted.rules,
            "signature": trust_signature,
        });
        let out = self
            .execute_command_on_active_session(build_decide_command_with_trust(
                &item.id,
                true,
                ts,
                &signature,
                Some(trust),
            )?)
            .await?;
        parse_ctl_result(&out)
    }

    /// Active temporary trusts on the host.
    pub async fn list_trusts(&self) -> Result<Vec<crate::auto_approval::AutoTrust>, SshCoreError> {
        use crate::approval_queue::{build_ctl_command, CtlSubcommand};
        let out = self
            .execute_command_on_active_session(build_ctl_command(CtlSubcommand::Trusts))
            .await?;
        Ok(crate::auto_approval::parse_trusts(&out))
    }

    pub async fn revoke_trust(&self, id: String) -> Result<(), SshCoreError> {
        let out = self
            .execute_command_on_active_session(crate::approval_queue::build_revoke_command(&id)?)
            .await?;
        crate::approval_queue::parse_ctl_result(&out)
    }

    pub async fn revoke_all_trusts(&self) -> Result<(), SshCoreError> {
        use crate::approval_queue::{build_ctl_command, CtlSubcommand};
        let out = self
            .execute_command_on_active_session(build_ctl_command(CtlSubcommand::RevokeAll))
            .await?;
        crate::approval_queue::parse_ctl_result(&out)
    }

    /// The last automatic decisions of the hook (newest last).
    pub async fn read_auto_log(
        &self,
    ) -> Result<Vec<crate::auto_approval::AutoLogEntry>, SshCoreError> {
        use crate::approval_queue::{build_ctl_command, CtlSubcommand};
        let out = self
            .execute_command_on_active_session(build_ctl_command(CtlSubcommand::AutoLog))
            .await?;
        Ok(crate::auto_approval::parse_auto_log(&out))
    }

    /// Dismisses an informational notice on the host.
    pub async fn acknowledge_notification(&self, id: String) -> Result<(), SshCoreError> {
        let cmd = crate::approval_queue::build_ack_command(&id)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        crate::approval_queue::parse_ctl_result(&out)
    }

    /// Creates the host queue directories and trusts this device's key for approvals.
    pub async fn install_approval_key(&self) -> Result<(), SshCoreError> {
        let public = crate::key_store::key_info(read_ok(&self.config).key.clone())?
            .ok_or(SshCoreError::NotConnected)?
            .public_key_openssh;
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
