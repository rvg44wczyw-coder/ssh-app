use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
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
             {{ printf 'TP_PATH=%s\\n' \"$r\"; tail -c 32000000 -- \"$r\" | LC_ALL=C grep -aEv '^\\{{\"type\":\"(api-request-blob|api-request|mode|atis-latch|ai-title|last-prompt|queue-operation|file-history-snapshot)\"' | tail -n 3000 | tail -c 4000000; }}"
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

    /// Reads what a session file gained since `cursor` (complete lines only), or its usual window
    /// when there is no cursor or the file was replaced or shortened (`reset`). The caller feeds
    /// the text to a `FeedState` and keeps the returned cursor. `None`: no such file.
    pub async fn read_remote_transcript_since(
        &self,
        remote_path: String,
        cursor: Option<crate::feed_read::FeedCursor>,
    ) -> Result<Option<crate::feed_read::FeedRead>, SshCoreError> {
        let cmd = crate::feed_read::build_delta_command(&remote_path, cursor)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        crate::feed_read::parse_delta_output(&out)
    }

    /// The subagents of a Claude Code session (newest first, at most 20, nesting ≤ 3).
    pub async fn list_subagents(
        &self,
        session_file: String,
    ) -> Result<Vec<crate::subagent_feed::SubagentInfo>, SshCoreError> {
        let cmd = crate::subagent_feed::build_list_command(&session_file)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::subagent_feed::parse_subagent_list(&out))
    }

    /// Like `read_remote_transcript_since` for one subagent of the session; feed the text to a
    /// `FeedState::new_subagent()`. The path is built here from the id, never taken from the host.
    pub async fn read_subagent_since(
        &self,
        session_file: String,
        agent_id: String,
        cursor: Option<crate::feed_read::FeedCursor>,
    ) -> Result<Option<crate::feed_read::FeedRead>, SshCoreError> {
        let file = crate::subagent_feed::subagent_file(&session_file, &agent_id)?;
        self.read_remote_transcript_since(file, cursor).await
    }

    /// The child conversations (subagents) of an Antigravity tab, read by the pinned host helper
    /// (`helper_sha256`) from agy's index. Empty when the helper is missing or there are none.
    pub async fn list_agy_subagents(
        &self,
        session_name: String,
        helper_sha256: String,
    ) -> Result<Vec<crate::subagent_feed::SubagentInfo>, SshCoreError> {
        let cmd = crate::subagent_feed::build_agy_list_command(&session_name, &helper_sha256)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        Ok(crate::subagent_feed::parse_agy_children(&out))
    }

    /// Reads a child conversation of an Antigravity tab by cursor (feed the text to a `FeedState`).
    /// `None`: the helper or the conversation is not there.
    pub async fn read_agy_subagent_since(
        &self,
        conversation_id: String,
        helper_sha256: String,
        cursor: Option<crate::feed_read::FeedCursor>,
    ) -> Result<Option<crate::feed_read::FeedRead>, SshCoreError> {
        let cmd =
            crate::subagent_feed::build_agy_read_command(&conversation_id, &helper_sha256, cursor)?;
        let out = self.execute_command_on_active_session(cmd).await?;
        crate::feed_read::parse_delta_output(&out)
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

    /// The text of this tab's tmux pane with its whole history (`capture-pane -e -p -J -S -`, styles kept), for the
    /// feed built from the screen. `known_sum` is the checksum of the previous answer: when the
    /// screen is unchanged the host answers `SAME` and nothing else crosses the network.
    /// Returns `(checksum, Some(text))`, or `(checksum, None)` when unchanged.
    pub async fn capture_session_screen(
        &self,
        session_name: String,
        known_sum: Option<String>,
    ) -> Result<ScreenCapture, SshCoreError> {
        self.capture_session_screen_in(session_name, known_sum, MultiplexerKind::Tmux)
            .await
    }

    /// Like `capture_session_screen`, for a tab running in tmux or Zellij (the focused pane).
    pub async fn capture_session_screen_in(
        &self,
        session_name: String,
        known_sum: Option<String>,
        kind: MultiplexerKind,
    ) -> Result<ScreenCapture, SshCoreError> {
        let program = crate::zellij_feed::mux_arg(kind, &session_name)?;
        let known = known_sum.unwrap_or_default();
        if !known.chars().all(|c| c.is_ascii_digit()) {
            return Err(SshCoreError::InvalidInput {
                reason: "Invalid checksum".to_string(),
            });
        }
        if program == "zellij" {
            let script = crate::hostprobe::login_sh_script(
                &crate::zellij_feed::zellij_capture_script(),
                &[
                    crate::tmux::shell_quote(session_name.trim()),
                    crate::tmux::shell_quote(&known),
                ],
                10,
            );
            let output = self.execute_command_on_active_session(script).await?;
            return match crate::screen_capture::parse_capture_output(&output)? {
                crate::screen_capture::CaptureAnswer::Same(checksum) => Ok(ScreenCapture {
                    checksum,
                    text: None,
                }),
                crate::screen_capture::CaptureAnswer::Text { checksum, text } => {
                    Ok(ScreenCapture {
                        checksum,
                        text: Some(text),
                    })
                }
                crate::screen_capture::CaptureAnswer::NoTmux => Err(SshCoreError::IoError {
                    reason: "The pane could not be read".to_string(),
                }),
            };
        }
        // tmux: one script waits for a stable frame on the host and sends it compressed. It runs
        // under plain `sh`; a host where that does not find tmux is asked through the login shell
        // from then on.
        let name = crate::tmux::shell_quote(session_name.trim());
        let quoted_known = crate::tmux::shell_quote(&known);
        let script = crate::screen_capture::capture_script("tmux");
        let plain = format!(
            "exec sh -c {} sh {} {}",
            crate::tmux::shell_quote(&script),
            name,
            quoted_known
        );
        let login = || {
            crate::hostprobe::login_sh_script(&script, &[name.clone(), quoted_known.clone()], 10)
        };
        let use_login = self
            .capture_via_login
            .load(std::sync::atomic::Ordering::Relaxed);
        let output = self
            .execute_command_on_active_session(if use_login { login() } else { plain })
            .await?;
        let answer = match crate::screen_capture::parse_capture_output(&output)? {
            crate::screen_capture::CaptureAnswer::NoTmux if !use_login => {
                self.capture_via_login
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                let again = self.execute_command_on_active_session(login()).await?;
                crate::screen_capture::parse_capture_output(&again)?
            }
            other => other,
        };
        match answer {
            crate::screen_capture::CaptureAnswer::Same(checksum) => Ok(ScreenCapture {
                checksum,
                text: None,
            }),
            crate::screen_capture::CaptureAnswer::Text { checksum, text } => Ok(ScreenCapture {
                checksum,
                text: Some(text),
            }),
            crate::screen_capture::CaptureAnswer::NoTmux => Err(SshCoreError::IoError {
                reason: "tmux was not found on the host".to_string(),
            }),
        }
    }

    /// The transcript of the agent running in this tab's own tmux session: Claude Code by the
    /// folder the process runs in, Antigravity by the conversation folder it has open (newest
    /// one if none is open). `None` when no agent runs in the pane; it never falls back to "the
    /// newest transcript on the host", which belongs to some other tab.
    pub async fn detect_session_transcript_path(
        &self,
        session_name: String,
    ) -> Result<Option<String>, SshCoreError> {
        self.detect_session_transcript_path_in(session_name, MultiplexerKind::Tmux)
            .await
    }

    /// Like `detect_session_transcript_path`, for a tab running in tmux or Zellij.
    pub async fn detect_session_transcript_path_in(
        &self,
        session_name: String,
        kind: MultiplexerKind,
    ) -> Result<Option<String>, SshCoreError> {
        let program = crate::zellij_feed::mux_arg(kind, &session_name)?;
        let script = crate::hostprobe::login_sh_script(
            &format!(
                "{}{}",
                crate::zellij_feed::roots_prefix(""),
                session_transcript_script()
            ),
            &[
                crate::tmux::shell_quote(session_name.trim()),
                program.to_string(),
            ],
            10,
        );
        let output = self.execute_command_on_active_session(script).await?;
        let path = output.trim();
        // The reader validates the path again before opening it.
        if path.is_empty() || path.contains('\n') || path.chars().any(char::is_control) {
            return Ok(None);
        }
        Ok(Some(path.to_string()))
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
}
