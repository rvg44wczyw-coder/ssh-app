use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Restarts the agent of a tab: the pane of the tmux session `session_name` is respawned with
    /// `command` (a launch command from `build_agent_launch_command`), the old process killed. The
    /// session stays, so the phone's attached terminal just shows the new start.
    pub async fn restart_session_command(
        &self,
        session_name: String,
        command: String,
    ) -> Result<(), SshCoreError> {
        crate::tmux::validate_session_name(&session_name)?;
        if command.trim().is_empty() || command.len() > 8000 || command.contains('\0') {
            return Err(SshCoreError::InvalidInput {
                reason: "Invalid launch command".to_string(),
            });
        }
        let script = crate::hostprobe::login_sh_script(
            RESTART_PANE_SCRIPT,
            &[
                crate::tmux::shell_quote(session_name.trim()),
                crate::tmux::shell_quote(&command),
            ],
            10,
        );
        let out = self.execute_command_on_active_session(script).await?;
        if out.lines().any(|l| l.trim() == "RESTARTED") {
            Ok(())
        } else {
            Err(SshCoreError::IoError {
                reason: "tmux did not restart the session".to_string(),
            })
        }
    }

    /// Sends a file picked on the phone (photo, video, any file) to the host over a new channel
    /// of this session: `permanent` keeps it in `~/.ssh-app/inbox/<id>-<name>`, otherwise it goes
    /// to a private folder in the host's temp directory (`$TMPDIR` or `/tmp`, which the system
    /// empties). Mode 0600, folder 0700. Returns the absolute path on the host, to put in a message to the agent.
    /// The name is reduced to `[A-Za-z0-9._-]`; at most `ATTACHMENT_MAX_BYTES`.
    /// `upload_id` is chosen by the caller; `cancel_upload` with the same id stops the transfer
    /// (error `Cancelled`) and the host keeps nothing of it.
    pub async fn upload_attachment(
        &self,
        src_local_path: String,
        file_name: String,
        permanent: bool,
        upload_id: String,
    ) -> Result<String, SshCoreError> {
        use tokio::io::AsyncReadExt;
        let io = |e: std::io::Error| SshCoreError::IoError {
            reason: e.to_string(),
        };
        let meta = tokio::fs::metadata(&src_local_path).await.map_err(io)?;
        if !meta.is_file() {
            return Err(SshCoreError::InvalidInput {
                reason: "Not a regular file".to_string(),
            });
        }
        if meta.len() > ATTACHMENT_MAX_BYTES {
            return Err(SshCoreError::TooLarge {
                limit: ATTACHMENT_MAX_BYTES,
            });
        }
        let stored = format!(
            "{:08x}-{}",
            rand::random::<u32>(),
            attachment_name(&file_name)
        );
        let cancel = self
            .upload_cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(upload_id.clone())
            .or_default()
            .clone();
        let _registered = UploadRegistration {
            map: Arc::clone(&self.upload_cancels),
            id: upload_id,
        };
        // Closed if this future is dropped (the user cancelled): the host then discards the part.
        let mut guard = crate::transport::CloseOnDrop(Some(
            self.exec_open(&attachment_command(&stored, permanent, meta.len()))
                .await?,
        ));
        let mut file = tokio::fs::File::open(&src_local_path).await.map_err(io)?;
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SshCoreError::Cancelled);
            }
            let n = file.read(&mut buf).await.map_err(io)?;
            if n == 0 {
                break;
            }
            let channel = guard.0.as_ref().ok_or(SshCoreError::NotConnected)?;
            channel
                .data(&buf[..n])
                .await
                .map_err(|e| SshCoreError::ChannelError {
                    reason: format!("Could not send the file: {e}"),
                })?;
        }
        let channel = guard.0.take().ok_or(SshCoreError::NotConnected)?;
        let _ = channel.eof().await;
        let out = crate::transport::collect_output(channel, Duration::from_secs(120)).await?;
        let text = out.text();
        match (out.exit_status, parse_attachment_path(&text)) {
            (Some(0), Some(path)) => Ok(path),
            _ => Err(SshCoreError::IoError {
                reason: "The host did not store the file".to_string(),
            }),
        }
    }

    /// Stops the upload started with this id (also one that has not begun yet).
    pub fn cancel_upload(&self, upload_id: String) {
        self.upload_cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(upload_id)
            .or_default()
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Asks the host to send one test notification through the relay.
    pub async fn host_push_test(
        &self,
        expected_sha256: String,
    ) -> Result<crate::push::HostPushResult, SshCoreError> {
        let cmd = crate::push::build_push_command(&expected_sha256, "push-test")?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::push::parse_result(&out))
    }

    /// Models the agent can be switched to, from the pinned helper on the host (`claude`,
    /// `antigravity` or `ollama`). Empty when the helper is not installed or knows none; for
    /// Claude the built-in aliases are returned then.
    pub async fn fetch_agent_models(
        &self,
        agent: String,
        expected_sha256: String,
    ) -> Result<Vec<crate::usage::AgentModelChoice>, SshCoreError> {
        crate::usage::validate_sha256(&expected_sha256)?;
        let cmd = crate::usage::build_models_command(&expected_sha256, &agent)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        if let Some(list) = crate::usage::parse_models(&out).filter(|l| !l.is_empty()) {
            return Ok(list);
        }
        // No usable helper (absent, replaced or older than this app): ask the agent's own command.
        if let Some(direct) = crate::usage::build_models_direct_command(&agent) {
            let out = self.execute_command_on_active_session(direct).await?;
            return Ok(crate::usage::parse_models_direct(&agent, &out));
        }
        Ok(if agent == "claude" {
            ["fable", "opus", "sonnet", "haiku"]
                .iter()
                .map(|a| crate::usage::AgentModelChoice {
                    id: a.to_string(),
                    label: format!("{a} (latest)"),
                })
                .collect()
        } else {
            Vec::new()
        })
    }

    /// Subscribes to live state changes of the agent sessions. One exec channel stays open (the
    /// `rpc` bridge to the host agent); each change is delivered to `listener`, and `on_closed` is
    /// called once when the stream ends (the helper is missing or replaced, the connection drops
    /// or `stop` was called). Nothing is polled; the channel is closed with the connection, so it
    /// costs nothing in the background.
    pub async fn watch_host_agent(
        &self,
        expected_sha256: String,
        listener: Box<dyn HostAgentListener>,
    ) -> Result<Arc<HostAgentWatch>, SshCoreError> {
        crate::usage::validate_sha256(&expected_sha256)?;
        let channel = self
            .exec_open(&crate::usage::build_agent_watch_command(&expected_sha256))
            .await?;
        let mut stream = crate::transport::ExecStream::new(channel);
        stream
            .write(b"{\"id\":1,\"m\":\"subscribe\"}\n".to_vec())
            .await?;
        // Return only once the agent has accepted the subscription, so no change that happens
        // after this call returns is missed.
        let mut pending = String::new();
        let acked = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match stream.next().await {
                    crate::transport::StreamEvent::Data(bytes) => {
                        pending.push_str(&String::from_utf8_lossy(&bytes));
                        if pending.len() > 8192 {
                            return false;
                        }
                        if pending.lines().any(crate::usage::is_subscribe_ack) {
                            return true;
                        }
                    }
                    crate::transport::StreamEvent::Exit(_) => {}
                    crate::transport::StreamEvent::End => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        if !acked {
            stream.finish().await;
            return Err(SshCoreError::IoError {
                reason: "The host agent did not accept the subscription".to_string(),
            });
        }
        let watch = Arc::new(HostAgentWatch {
            stop: tokio::sync::Notify::new(),
        });
        let task_watch = Arc::clone(&watch);
        crate::session::RUNTIME.spawn(async move {
            // Lines that arrived together with the acknowledgement.
            while let Some(nl) = pending.find('\n') {
                let line: String = pending.drain(..=nl).collect();
                if let Some(ev) = crate::usage::parse_agent_event(&line) {
                    listener.on_event(ev);
                }
            }
            loop {
                let step = tokio::select! {
                    s = stream.next() => s,
                    () = task_watch.stop.notified() => break,
                };
                match step {
                    crate::transport::StreamEvent::Data(bytes) => {
                        pending.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(nl) = pending.find('\n') {
                            let line: String = pending.drain(..=nl).collect();
                            if let Some(ev) = crate::usage::parse_agent_event(&line) {
                                listener.on_event(ev);
                            }
                        }
                        if pending.len() > 8192 {
                            pending.clear();
                        }
                    }
                    crate::transport::StreamEvent::Exit(_) => {}
                    crate::transport::StreamEvent::End => break,
                }
            }
            stream.finish().await;
            listener.on_closed();
        });
        Ok(watch)
    }
}

/// Takes the upload's entry out of the table when the upload ends, however it ends.
struct UploadRegistration {
    map: Arc<
        std::sync::Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    >,
    id: String,
}

impl Drop for UploadRegistration {
    fn drop(&mut self) {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}
