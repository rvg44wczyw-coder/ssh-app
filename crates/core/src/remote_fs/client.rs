use super::*;

#[derive(uniffi::Object)]
pub struct RemoteFsClient {
    config: RemoteServerConfig,
    conn: tokio::sync::Mutex<Option<ClientHandle>>,
}

impl RemoteFsClient {
    async fn open(
        &self,
        command: &str,
    ) -> Result<russh::Channel<russh::client::Msg>, SshCoreError> {
        for attempt in 0..2 {
            let mut slot = self.conn.lock().await;
            if slot.is_none() {
                let c = &self.config;
                let conn = connect_authenticated(
                    &Endpoint {
                        host: &c.host,
                        port: c.port,
                        username: &c.username,
                        key: &c.key,
                        pinned_host_key: c.pinned_host_key.as_deref(),
                    },
                    Duration::from_secs(10),
                    2,
                )
                .await?;
                *slot = Some(conn.handle);
            }
            if let Some(h) = slot.as_ref() {
                match exec_start(h, command).await {
                    Ok(ch) => return Ok(ch),
                    Err(e) if attempt == 1 => return Err(e),
                    Err(_) => *slot = None,
                }
            }
        }
        Err(SshCoreError::NotConnected)
    }

    async fn run(&self, command: &str) -> Result<crate::transport::ExecOutput, SshCoreError> {
        let ch = self.open(command).await?;
        collect_output(ch, DEFAULT_EXEC_TIMEOUT).await
    }

    async fn resolve(&self, path: &str) -> Result<String, SshCoreError> {
        let out = self.run(&build_resolve_path_command(path)).await?;
        let resolved = out.text();
        let resolved = resolved.trim();
        if resolved.is_empty() {
            return Err(SshCoreError::FileNotFound {
                path: path.to_string(),
            });
        }
        clean_allowed(resolved)
    }

    /// `path` with its folder resolved (symlinks followed) and checked again: a link inside an
    /// allowed folder must not let a write land in a blocked one.
    async fn resolved_child(&self, path: &str) -> Result<String, SshCoreError> {
        let parent = self.resolve(&parent_of(path)).await?;
        let child = if parent == "/" {
            format!("/{}", name_of(path))
        } else {
            format!("{parent}/{}", name_of(path))
        };
        check_allowed(&child)?;
        Ok(child)
    }

    async fn run_checked(&self, script: String, path: &str) -> Result<(), SshCoreError> {
        let out = self.run(&format!("sh -c {}", shell_quote(&script))).await?;
        match out.exit_status {
            Some(0) => Ok(()),
            Some(c) => Err(exit_error(c, path)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not report a result".to_string(),
            }),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RemoteFsClient {
    /// A pinned host key is mandatory: this client never trusts a key on first use.
    #[uniffi::constructor]
    pub fn new(config: RemoteServerConfig) -> Result<Arc<Self>, SshCoreError> {
        if config.pinned_host_key.as_deref().is_none_or(str::is_empty) {
            return Err(invalid("The host key must be trusted in the app first"));
        }
        Ok(Arc::new(RemoteFsClient {
            config,
            conn: tokio::sync::Mutex::new(None),
        }))
    }

    pub async fn close(&self) {
        if let Some(h) = self.conn.lock().await.take() {
            let _ = h
                .disconnect(russh::Disconnect::ByApplication, "", "English")
                .await;
        }
    }

    /// `$HOME` of the login user: the root of the server in the Files app.
    pub async fn home_directory(&self) -> Result<String, SshCoreError> {
        let out = self.run("printf 'FS_HOME=%s\\n' \"$HOME\"").await?;
        let text = out.text();
        let home = text
            .lines()
            .find_map(|l| l.strip_prefix("FS_HOME="))
            .ok_or_else(|| SshCoreError::IoError {
                reason: "The host did not report the home folder".to_string(),
            })?;
        let home = clean_path(home)?;
        validate_remote_dir_path(&home)?;
        Ok(home)
    }

    pub async fn stat_remote(&self, path: String) -> Result<RemoteEntry, SshCoreError> {
        let path = clean_path(&path)?;
        let script = stat_script();
        let out = self
            .run(&format!(
                "sh -c {} sh {}",
                shell_quote(&script),
                shell_quote(&path)
            ))
            .await?;
        let mut e = parse_stat(&out.text()).map_err(|e| match e {
            SshCoreError::FileNotFound { .. } => SshCoreError::FileNotFound { path: path.clone() },
            e => e,
        })?;
        // The root of a share may be asked for by a link; the user-facing name stays the requested one.
        if path == "/" {
            e.name = "/".to_string();
        }
        Ok(e)
    }

    pub async fn list_remote_entries(
        &self,
        path: String,
        include_hidden: bool,
    ) -> Result<Vec<RemoteEntry>, SshCoreError> {
        let path = clean_allowed(&path)?;
        let script = list_script();
        let out = self
            .run(&format!(
                "sh -c {} sh {}",
                shell_quote(&script),
                shell_quote(&path)
            ))
            .await?;
        parse_listing(&out.text(), include_hidden).map(|(_, e)| e)
    }

    /// Streams a regular file to `dest_local_path` (written as `<dest>.part`, renamed when complete).
    pub async fn download_remote_file(
        &self,
        path: String,
        dest_local_path: String,
        max_bytes: u64,
        progress: Box<dyn TransferProgress>,
    ) -> Result<TransferResult, SshCoreError> {
        let requested = clean_allowed(&path)?;
        let resolved = self.resolve(&requested).await?;
        let meta = self.stat_remote(resolved.clone()).await?;
        if meta.kind != RemoteFileKind::File {
            return Err(SshCoreError::NotARegularFile { path: resolved });
        }
        if meta.size > max_bytes {
            return Err(SshCoreError::TooLarge { limit: max_bytes });
        }
        let part = format!("{dest_local_path}.part");
        let result = self
            .stream_to_file(&resolved, &part, max_bytes, progress.as_ref())
            .await;
        match result {
            Ok(n) => {
                tokio::fs::rename(&part, &dest_local_path)
                    .await
                    .map_err(|e| SshCoreError::IoError {
                        reason: e.to_string(),
                    })?;
                Ok(TransferResult { bytes: n })
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    /// Sends a local file to `dest_path`; without `overwrite` an existing file is never touched.
    pub async fn upload_remote_file(
        &self,
        src_local_path: String,
        dest_path: String,
        overwrite: bool,
        progress: Box<dyn TransferProgress>,
    ) -> Result<TransferResult, SshCoreError> {
        let dest = self.resolved_child(&clean_allowed(&dest_path)?).await?;
        // Answer "exists" at once instead of streaming into a command that has already stopped.
        if !overwrite && self.stat_remote(dest.clone()).await.is_ok() {
            return Err(SshCoreError::AlreadyExists { path: dest });
        }
        let mut file =
            tokio::fs::File::open(&src_local_path)
                .await
                .map_err(|e| SshCoreError::IoError {
                    reason: e.to_string(),
                })?;
        let mut stream = ExecStream::new(self.open(&upload_command(&dest, overwrite)).await?);
        let mut sent = 0u64;
        let mut next_report = PROGRESS_STEP;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if progress.is_cancelled() {
                return Err(SshCoreError::Cancelled);
            }
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| SshCoreError::IoError {
                    reason: e.to_string(),
                })?;
            if n == 0 {
                break;
            }
            // A host command that exited early (target exists, no permission) stops reading, and
            // the send window never reopens: ask for its exit status instead of waiting forever.
            let sent_ok = matches!(
                tokio::time::timeout(WRITE_STALL, stream.write(buf[..n].to_vec())).await,
                Ok(Ok(()))
            );
            if !sent_ok {
                return Err(
                    match tokio::time::timeout(Duration::from_secs(5), stream.exit_status()).await {
                        Ok(Some(c)) if c != 0 => exit_error(c, &dest),
                        _ => SshCoreError::Timeout,
                    },
                );
            }
            sent += n as u64;
            if sent >= next_report {
                progress.on_progress(sent);
                next_report = sent + PROGRESS_STEP;
            }
        }
        stream.eof().await;
        let mut exit = None;
        loop {
            match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
                Ok(StreamEvent::Exit(c)) => exit = Some(c),
                Ok(StreamEvent::Data(_)) => {}
                Ok(StreamEvent::End) => break,
                Err(_) => return Err(SshCoreError::Timeout),
            }
        }
        stream.finish().await;
        match exit {
            Some(0) => {
                progress.on_progress(sent);
                Ok(TransferResult { bytes: sent })
            }
            Some(c) => Err(exit_error(c, &dest)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not confirm the upload".to_string(),
            }),
        }
    }

    pub async fn make_remote_dir(&self, path: String) -> Result<(), SshCoreError> {
        let p = self.resolved_child(&clean_allowed(&path)?).await?;
        self.run_checked(format!("mkdir -- {}", shell_quote(&p)), &p)
            .await
    }

    /// Renames or moves `from` to `to`; an existing target is never replaced.
    pub async fn rename_remote(&self, from: String, to: String) -> Result<(), SshCoreError> {
        let (a, b) = (clean_allowed(&from)?, clean_allowed(&to)?);
        if a == "/" || b == "/" {
            return Err(invalid("Cannot move the root"));
        }
        let (a, b) = (
            self.resolved_child(&a).await?,
            self.resolved_child(&b).await?,
        );
        let script = format!(
            "[ -e {a} ] || exit 3; [ ! -e {b} ] && [ ! -L {b} ] || exit 4; mv -- {a} {b}",
            a = shell_quote(&a),
            b = shell_quote(&b)
        );
        self.run_checked(script, &b).await
    }

    /// Deletes a file or an empty folder. Never recursive.
    pub async fn delete_remote(&self, path: String) -> Result<(), SshCoreError> {
        let p = clean_allowed(&path)?;
        if p == "/" {
            return Err(invalid("Cannot delete the root"));
        }
        let p = self.resolved_child(&p).await?;
        let q = shell_quote(&p);
        self.run_checked(
            format!("if [ -d {q} ] && [ ! -L {q} ]; then rmdir -- {q}; else rm -- {q}; fi"),
            &p,
        )
        .await
    }
}

impl RemoteFsClient {
    async fn stream_to_file(
        &self,
        path: &str,
        part: &str,
        max_bytes: u64,
        progress: &dyn TransferProgress,
    ) -> Result<u64, SshCoreError> {
        let mut file = tokio::fs::File::create(part)
            .await
            .map_err(|e| SshCoreError::IoError {
                reason: e.to_string(),
            })?;
        let mut stream = ExecStream::new(self.open(&download_command(path)).await?);
        let mut written = 0u64;
        let mut next_report = PROGRESS_STEP;
        let mut exit = None;
        loop {
            if progress.is_cancelled() {
                return Err(SshCoreError::Cancelled);
            }
            match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
                Ok(StreamEvent::Data(d)) => {
                    written += d.len() as u64;
                    if written > max_bytes {
                        return Err(SshCoreError::TooLarge { limit: max_bytes });
                    }
                    file.write_all(&d)
                        .await
                        .map_err(|e| SshCoreError::IoError {
                            reason: e.to_string(),
                        })?;
                    if written >= next_report {
                        progress.on_progress(written);
                        next_report = written + PROGRESS_STEP;
                    }
                }
                Ok(StreamEvent::Exit(c)) => exit = Some(c),
                Ok(StreamEvent::End) => break,
                Err(_) => return Err(SshCoreError::Timeout),
            }
        }
        stream.finish().await;
        file.flush().await.map_err(|e| SshCoreError::IoError {
            reason: e.to_string(),
        })?;
        match exit {
            Some(0) => {
                progress.on_progress(written);
                Ok(written)
            }
            Some(c) => Err(exit_error(c, path)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not confirm the end of the file".to_string(),
            }),
        }
    }
}
