//! Host-side features of an active session: transcripts, history, telemetry, files, agent hooks,
//! permissions and approvals. Every method is a thin, validated command on top of the session's
//! exec primitive (`SshSessionHandle::exec_with_timeout`); the pure parts live in their own modules.

use crate::error::SshCoreError;
use crate::multiplexer::MultiplexerKind;
use crate::session::{read_ok, SshSessionHandle};
use crate::transport::DEFAULT_EXEC_TIMEOUT;
use std::sync::Arc;
use std::time::Duration;

/// The answer of `capture_session_screen`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScreenCapture {
    pub checksum: String,
    /// `None`: the screen is the same as the one with the known checksum.
    pub text: Option<String>,
}

/// Restarts what runs in a tmux session's pane with a new command (`respawn-pane -k`): the session,
/// its window and the attached clients stay, the old process is killed.
const RESTART_PANE_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
tmux respawn-pane -k -t "=$1:" "$2" 2>&1 || exit 1
echo RESTARTED"#;

/// Receives the live state changes of `watch_host_agent`.
#[uniffi::export(callback_interface)]
pub trait HostAgentListener: Send + Sync {
    fn on_event(&self, event: crate::usage::HostAgentEvent);
    /// The stream ended; no further events arrive.
    fn on_closed(&self);
}

/// Handle of a running `watch_host_agent` stream.
#[derive(uniffi::Object)]
pub struct HostAgentWatch {
    stop: tokio::sync::Notify,
}

#[uniffi::export]
impl HostAgentWatch {
    /// Ends the stream (idempotent); `on_closed` follows.
    pub fn stop(&self) {
        self.stop.notify_one();
    }
}

impl SshSessionHandle {
    async fn read_remote_config(&self, path: &str) -> Result<String, SshCoreError> {
        self.execute_command_on_active_session(format!(
            "cat {} 2>/dev/null || true",
            crate::agents::remote_path_expr(path)
        ))
        .await
    }

    /// Writes through symlinks, keeps the original mode (new files `0600`), backs up once.
    /// `previous` is what the caller read and merged into `content`: if the file on the host
    /// changed since (the agent or the user edited it), nothing is written.
    async fn write_remote_config(
        &self,
        path: &str,
        previous: &str,
        content: &str,
    ) -> Result<(), SshCoreError> {
        let cmd = build_write_config_command(path, previous, content);
        let out = self.execute_command_on_active_session(cmd).await?;
        let last = out.lines().last().unwrap_or("").trim();
        match last {
            "CONFIG_WRITTEN" => Ok(()),
            "CONFIG_CHANGED" => Err(SshCoreError::IoError {
                reason: "The config file changed on the host while you were editing it; nothing was written. Reopen and try again.".to_string(),
            }),
            _ => Err(SshCoreError::IoError {
                reason: format!(
                    "Writing the config file failed on the host: {}",
                    crate::approval_queue::display_safe(&out.chars().take(200).collect::<String>())
                ),
            }),
        }
    }
}

const TRANSCRIPT_DETECT_SCRIPT: &str = r#"TARGET_DIR="$1"
if [ -d "$HOME/.gemini/antigravity-cli/brain" ]; then
  LATEST=$(ls -td "$HOME/.gemini/antigravity-cli/brain"/* 2>/dev/null | head -n 1)
  if [ -n "$LATEST" ] && [ -f "$LATEST/.system_generated/logs/transcript.jsonl" ]; then
    echo "$LATEST/.system_generated/logs/transcript.jsonl"
    exit 0
  fi
fi
if [ -d "$HOME/.claude/projects" ]; then
  LATEST_CLAUDE=$(ls -td "$HOME/.claude/projects"/*/*.jsonl 2>/dev/null | head -n 1)
  if [ -n "$LATEST_CLAUDE" ] && [ -f "$LATEST_CLAUDE" ]; then
    echo "$LATEST_CLAUDE"
    exit 0
  fi
fi
if [ -n "$TARGET_DIR" ] && [ -f "$TARGET_DIR/.agents/transcript.jsonl" ]; then
  echo "$TARGET_DIR/.agents/transcript.jsonl"
  exit 0
fi
if [ -n "$TARGET_DIR" ] && [ -f "$TARGET_DIR/transcript.jsonl" ]; then
  echo "$TARGET_DIR/transcript.jsonl"
  exit 0
fi"#;

/// Host command that replaces the config file only if it still holds `previous`; prints
/// `CONFIG_WRITTEN` or `CONFIG_CHANGED` as its last line.
pub(crate) fn build_write_config_command(path: &str, previous: &str, content: &str) -> String {
    use sha2::{Digest, Sha256};
    let expected = hex::encode(Sha256::digest(previous.as_bytes()));
    let p = crate::agents::remote_path_expr(path);
    let body = crate::tmux::shell_quote(content);
    format!(
            "set -e; p={p}; mkdir -p \"$(dirname \"$p\")\"; \
             if [ -L \"$p\" ]; then p=$(realpath -- \"$p\" 2>/dev/null || readlink -f -- \"$p\"); fi; \
             if [ -f \"$p\" ] && [ ! -e \"$p.ssh-app.bak\" ]; then cp -p \"$p\" \"$p.ssh-app.bak\"; fi; \
             t=\"$p.ssh-app.tmp\"; rm -f \"$t\"; umask 077; set -C; printf %s {body} > \"$t\"; set +C; \
             if [ -f \"$p\" ]; then chmod --reference=\"$p\" \"$t\" 2>/dev/null || chmod \"$(stat -f %Lp \"$p\" 2>/dev/null || echo 600)\" \"$t\"; fi; \
             h=$( {{ cat \"$p\" 2>/dev/null || true; }} | {{ shasum -a 256 2>/dev/null || sha256sum; }} | cut -d' ' -f1 ); \
             if [ \"$h\" != '{expected}' ]; then rm -f \"$t\"; echo CONFIG_CHANGED; exit 0; fi; \
             mv \"$t\" \"$p\" && echo CONFIG_WRITTEN"
        )
}

impl SshSessionHandle {
    /// `chat_host_ollama` against another port (tests run a fake server).
    pub(crate) async fn chat_ollama_on_port(
        &self,
        port: u16,
        model: String,
        system_prompt: String,
        messages: Vec<crate::ai::ChatMessage>,
    ) -> Result<String, SshCoreError> {
        let payload = crate::ai::build_ollama_chat_payload(&model, &system_prompt, &messages)?;
        let curl_cmd = crate::hostprobe::cancellable_sh(&format!(
            "curl -s --max-time 290 -X POST http://127.0.0.1:{port}/api/chat -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload)
        ));
        let raw = self
            .exec_with_timeout(&curl_cmd, Duration::from_secs(300))
            .await?;
        crate::ai::parse_ollama_chat_response(&raw)
    }

    /// `query_host_ollama` against another port (tests run a fake server; the product uses 11434).
    pub(crate) async fn query_ollama_on_port(
        &self,
        port: u16,
        model: String,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<crate::ai::AiCommandSuggestion, SshCoreError> {
        let full_prompt = format!("{system_prompt}\n\n{user_prompt}");
        let payload = serde_json::json!({
            "model": model,
            "prompt": full_prompt,
            "stream": false
        });
        let payload_str = payload.to_string();

        let curl_cmd = crate::hostprobe::cancellable_sh(&format!(
            "curl -s --max-time 290 -X POST http://127.0.0.1:{port}/api/generate -H 'Content-Type: application/json' -d {}",
            crate::tmux::shell_quote(&payload_str)
        ));

        // Local models can be slow, but not forever.
        let raw_response = self
            .exec_with_timeout(&curl_cmd, Duration::from_secs(300))
            .await?;
        let generated_text = crate::ai::parse_ollama_generate_response(&raw_response)?;
        crate::ai::parse_ai_response(&generated_text)
    }
}

/// Largest attachment the feed uploads (a phone video is the realistic upper end).
const ATTACHMENT_MAX_BYTES: u64 = 200 * 1024 * 1024;

/// A file name safe for the host's shell and file system: `[A-Za-z0-9._-]`, no leading dots,
/// at most 80 characters (the extension is kept), never empty.
pub(crate) fn attachment_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    clean = clean.trim_start_matches('.').to_string();
    if clean.chars().count() > 80 {
        let ext: String = match clean.rfind('.') {
            Some(i) if clean.len() - i <= 12 => clean[i..].to_string(),
            _ => String::new(),
        };
        let keep = 80 - ext.chars().count();
        clean = clean.chars().take(keep).collect::<String>() + &ext;
    }
    if clean.is_empty() {
        "file".to_string()
    } else {
        clean
    }
}

/// The host side of an upload: the file arrives on stdin, is written next to its place and moved
/// in, and the path is printed. The folder is `~/.ssh-app/inbox` (`permanent`) or
/// `${TMPDIR:-/tmp}/ssh-app-<uid>` (temporary, left to the system to empty); it must be a real
/// directory of the user's own, never a link or somebody else's. `stored` is already reduced to
/// safe characters. The file is moved into place only when exactly `size` bytes arrived: an upload
/// that was cancelled or cut leaves nothing behind.
pub(crate) fn attachment_command(stored: &str, permanent: bool, size: u64) -> String {
    let dir = if permanent {
        "d=\"$HOME/.ssh-app/inbox\"".to_string()
    } else {
        "d=\"${TMPDIR:-/tmp}\"; d=\"${d%/}/ssh-app-$(id -u)\"".to_string()
    };
    format!(
        "sh -c 'umask 077; {dir}; mkdir -p \"$d\" || exit 1; [ -d \"$d\" ] && [ ! -L \"$d\" ] && [ -O \"$d\" ] || exit 1; \
         f=\"$d/{stored}\"; \
         cat > \"$f.part\" || {{ rm -f \"$f.part\"; exit 1; }}; \
         [ \"$(wc -c < \"$f.part\" | tr -d \" \")\" = \"{size}\" ] || {{ rm -f \"$f.part\"; exit 1; }}; \
         mv \"$f.part\" \"$f\" || exit 1; \
         printf \"ATTACH_PATH=%s\\n\" \"$f\"'"
    )
}

pub(crate) fn parse_attachment_path(out: &str) -> Option<String> {
    out.lines()
        .rev()
        .find_map(|l| l.strip_prefix("ATTACH_PATH="))
        .map(|p| p.trim().to_string())
        .filter(|p| p.starts_with('/'))
}

mod approvals;
mod docker_ops;
mod feed_watch;
mod sandbox;
pub use feed_watch::{FeedListener, FeedWatch};
mod git;
mod helper_install;
mod hooks;
mod host_info;
mod ollama;
mod session_io;
mod skills;
mod statusline_push;
#[cfg(test)]
mod tests;
mod transcript;

/// Entry points for the loopback tests (fake Ollama on another port); not part of the UniFFI surface.
#[cfg(feature = "test-support")]
impl SshSessionHandle {
    pub async fn test_chat_ollama_on_port(
        &self,
        port: u16,
        model: String,
        system_prompt: String,
        messages: Vec<crate::ai::ChatMessage>,
    ) -> Result<String, SshCoreError> {
        self.chat_ollama_on_port(port, model, system_prompt, messages)
            .await
    }

    pub async fn test_query_ollama_on_port(
        &self,
        port: u16,
        model: String,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<crate::ai::AiCommandSuggestion, SshCoreError> {
        self.query_ollama_on_port(port, model, system_prompt, user_prompt)
            .await
    }
}

/// The host script that finds the session of the agent running in a tab (built from the agent registry
/// and the `SessionSource`s).
pub(crate) fn session_transcript_script() -> String {
    crate::session_source::locate_script()
}
