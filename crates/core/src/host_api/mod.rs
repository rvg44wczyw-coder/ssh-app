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

const CAPTURE_SCREEN_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
out=$(tmux capture-pane -e -p -J -S - -t "=$1:" 2>/dev/null) || exit 1
sum=$(printf '%s' "$out" | cksum | cut -d' ' -f1)
if [ "$sum" = "$2" ]; then echo "SAME $sum"; else echo "SUM $sum"; printf '%s\n' "$out"; fi
exit 0"#;

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

/// The part of the transcript detection after the pane's processes are known: `$roots` (space
/// separated pids; see `zellij_feed::roots_prefix`) are the processes the walk may start from.
/// `$1` is a validated session name. Prints one path or nothing.
pub(crate) const SESSION_TRANSCRIPT_BODY: &str = r#"found=$(ps -Ao pid=,ppid=,args= 2>/dev/null | awk -v roots=" $roots " '
  { id=$1; par=$2; $1=""; $2=""; sub(/^[ \t]+/, ""); pp[id]=par; cmd[id]=$0 }
  END { for (p in pp) { q=p; d=0; while (q != "" && q != 0 && d < 40) { if (index(roots, " " q " ")) {
      c=cmd[p]; n=split(c, w, " ")
      for (i=1; i<=n && i<=4; i++) {
        b=w[i]; sub(/.*\//, "", b); sub(/\.(js|mjs|py)$/, "", b); b=tolower(b)
        if (i > 1 && w[1] !~ /(node|bun|deno|python[0-9.]*|sh|bash|zsh)$/) break
        if (b == "claude") { print "claude " p; exit }
        if (b == "agy" || b == "antigravity") { print "agy " p; exit }
      }
      if (c ~ /@anthropic-ai\/claude-code|\/claude-code\//) { print "claude " p; exit }
      if (c ~ /\/antigravity\//) { print "agy " p; exit }
      break } q=pp[q]; d++ } } }')
kind=${found%% *}; apid=${found##* }
case "$apid" in ""|*[!0-9]*) exit 0;; esac
# The agent's own session id, as the hooks noted it for this tmux session (~/.ssh-app/run/sessions):
# it names the file exactly, even when the same agent runs in the same folder on the laptop.
sid=""; bts=-1
for r in "$HOME"/.ssh-app/run/sessions/*.json; do
  [ -f "$r" ] || continue
  grep -q "\"tmux\":\"$1\"" "$r" 2>/dev/null || continue
  grep -q "\"agent\":\"$( [ "$kind" = agy ] && echo antigravity || echo claude )\"" "$r" 2>/dev/null || continue
  ts=$(sed -n 's/.*"ts":\([0-9]*\).*/\1/p' "$r" | head -n 1)
  if [ "${ts:-0}" -gt "$bts" ]; then
    bts=${ts:-0}; sid=$(sed -n 's/.*"session":"\([^"]*\)".*/\1/p' "$r" | head -n 1)
  fi
done
case "$sid" in ""|*[!A-Za-z0-9_-]*) sid="";; esac
cwd=$(readlink "/proc/$apid/cwd" 2>/dev/null)
[ -n "$cwd" ] || cwd=$(lsof -a -p "$apid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)
case "$kind" in
  claude)
    [ -n "$cwd" ] || exit 0
    slug=$(printf '%s' "$cwd" | sed 's/[^A-Za-z0-9]/-/g')
    # Claude keeps its files under $CLAUDE_CONFIG_DIR (the agent's own, read from its environment)
    # or ~/.claude; accounts set up as ~/.claude-<name> are looked at too. The newest file wins.
    cdir=$(ps eww -p "$apid" 2>/dev/null | tr ' ' '\n' | sed -n 's/^CLAUDE_CONFIG_DIR=//p' | head -n 1)
    f=""
    if [ -n "$sid" ]; then
      for d in "$cdir" "$HOME/.claude" "$HOME"/.claude-*; do
        [ -n "$d" ] || continue
        c=$(ls "$d"/projects/*/"$sid".jsonl 2>/dev/null | head -n 1)
        [ -f "$c" ] && f=$c && break
      done
      [ -f "$f" ] && { echo "$f"; exit 0; }
    fi
    for d in "$cdir" "$HOME/.claude" "$HOME"/.claude-*; do
      [ -n "$d" ] || continue
      c=$(ls -t "$d/projects/$slug"/*.jsonl 2>/dev/null | head -n 1)
      [ -f "$c" ] && { [ -z "$f" ] || [ "$c" -nt "$f" ]; } && f=$c
    done
    [ -f "$f" ] && echo "$f";;
  agy)
    d=""
    [ -n "$sid" ] && [ -d "$HOME/.gemini/antigravity-cli/brain/$sid" ] && d="$HOME/.gemini/antigravity-cli/brain/$sid"
    [ -n "$d" ] || d=$(ls -l "/proc/$apid/fd" 2>/dev/null | grep -o "/[^ ]*/brain/[^/]*" | head -n 1)
    [ -n "$d" ] || d=$(lsof -p "$apid" -Fn 2>/dev/null | sed -n 's/^n//p' | grep -o "^.*/brain/[^/]*" | head -n 1)
    [ -n "$d" ] || d=$(ls -td "$HOME/.gemini/antigravity-cli/brain"/* 2>/dev/null | head -n 1)
    f="$d/.system_generated/logs/transcript.jsonl"
    [ -n "$d" ] && [ -f "$f" ] && echo "$f";;
esac
exit 0"#;

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
mod git;
mod helper_install;
mod hooks;
mod host_info;
mod ollama;
mod session_io;
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
